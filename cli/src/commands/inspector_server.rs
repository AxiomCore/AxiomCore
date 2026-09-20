use std::{
    collections::BTreeMap,
    convert::Infallible,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use axiom_lib::{
    application_change::semantic_diff,
    application_evidence::{ApplicationEvidence, AxiomQuery, EvidenceIndex, EvidenceNodeKind},
    live_inspection::{
        InspectorConnection, LiveCausalEvent, LiveCommand, LiveCommandKind, LiveHandshake,
        LiveHandshakeResponse, LiveSelection, LiveStateEvent, LiveTargetSnapshot,
        LIVE_COMMAND_FORMAT, LIVE_CONNECTION_FORMAT, LIVE_HANDSHAKE_FORMAT,
    },
    question::{answer_question, plan_question, unsupported_answer, QuestionRequest},
    runtime_evidence::RuntimeSession,
};
use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Query, State},
    http::{header, HeaderMap, Response, StatusCode},
    response::{sse::Event as SseEvent, IntoResponse, Sse},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, RwLock};
use tower_http::cors::{Any, CorsLayer};
use uuid::Uuid;

use super::{inspector::InspectorPlanner, inspector_dashboard, inspector_jev, inspector_runtime};

#[derive(Clone)]
struct ServerState {
    token: Arc<str>,
    origin: Arc<str>,
    workspace: Arc<PathBuf>,
    evidence: Arc<RwLock<ApplicationEvidence>>,
    runtime: Arc<RwLock<Option<RuntimeSession>>>,
    generation: Arc<RwLock<u64>>,
    updated_ms: Arc<RwLock<u128>>,
    source_error: Arc<RwLock<Option<String>>>,
    live: Arc<RwLock<LiveStore>>,
    updates: broadcast::Sender<String>,
    allow_remote_planner: bool,
    jev_intent_threshold: f64,
    jev_selector_threshold: f64,
}

#[derive(Default)]
struct LiveStore {
    targets: BTreeMap<String, LiveTargetSnapshot>,
    commands: Vec<LiveCommand>,
    next_command: u64,
}

#[derive(Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SourceQuery {
    token: Option<String>,
    node_id: String,
    #[serde(default)]
    evidence_index: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LiveQuery {
    token: Option<String>,
    target: Option<String>,
    session_id: Option<String>,
    after: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommandRequest {
    target: String,
    session_id: String,
    graph_revision: String,
    kind: LiveCommandKind,
    semantic_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QuestionApiRequest {
    question: Option<String>,
    request: Option<QuestionRequest>,
    #[serde(default)]
    planner: Option<InspectorPlanner>,
}

pub(super) async fn serve(
    workspace: &Path,
    port: u16,
    no_open: bool,
    api_only: bool,
    allow_remote_planner: bool,
    jev_intent_threshold: f64,
    jev_selector_threshold: f64,
) -> Result<()> {
    let workspace = fs::canonicalize(workspace)
        .with_context(|| format!("resolve Inspector workspace {}", workspace.display()))?;
    let workspace = if workspace.is_file() {
        workspace.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        workspace
    };
    let evidence = super::inspector::load(&workspace)?;
    let runtime = inspector_runtime::load_latest(&workspace)?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
    let address = listener.local_addr()?;
    let origin = format!("http://{address}");
    let token = random_token();
    let (updates, _) = broadcast::channel(32);
    let initial_graph_revision = evidence.graph_revision.clone();
    let state = ServerState {
        token: Arc::from(token.as_str()),
        origin: Arc::from(origin.as_str()),
        workspace: Arc::new(workspace.clone()),
        evidence: Arc::new(RwLock::new(evidence)),
        runtime: Arc::new(RwLock::new(runtime)),
        generation: Arc::new(RwLock::new(1)),
        updated_ms: Arc::new(RwLock::new(now_ms())),
        source_error: Arc::new(RwLock::new(None)),
        live: Arc::new(RwLock::new(LiveStore::default())),
        updates,
        allow_remote_planner,
        jev_intent_threshold,
        jev_selector_threshold,
    };
    let mut router = Router::new()
        .route("/api/v1/status", get(api_status))
        .route("/api/v1/evidence", get(api_evidence))
        .route("/api/v1/source", get(api_source))
        .route("/api/v1/query", post(api_query))
        .route("/api/v1/questions", post(api_question))
        .route("/api/v1/runtime", get(api_runtime))
        .route("/api/v1/runtime/sessions", post(api_ingest_runtime))
        .route("/api/v1/changes", get(api_changes))
        .route("/api/v1/live", get(api_live))
        .route("/api/v1/live/handshake", post(api_live_handshake))
        .route("/api/v1/live/selection", post(api_live_selection))
        .route("/api/v1/live/state", post(api_live_state))
        .route("/api/v1/live/causal", post(api_live_causal))
        .route(
            "/api/v1/live/commands",
            get(api_live_commands).post(api_live_command),
        )
        .route("/api/v1/events", get(api_events));
    if !api_only {
        router = router
            .route("/", get(index))
            .route("/assets/inspector.css", get(styles))
            .route("/assets/inspector.js", get(script));
    }
    let router = router
        // Runtime audit batches can be larger than a question. Keep a hard
        // process boundary while preserving the existing ingestion workflow.
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .with_state(state.clone());
    tokio::spawn(watch_workspace(state));
    let connection_path = workspace.join(".axiom/inspector/live-connection.json");
    let connection = InspectorConnection {
        format: LIVE_CONNECTION_FORMAT.into(),
        endpoint: origin.clone(),
        token: token.clone(),
        graph_revision: initial_graph_revision,
        process_id: std::process::id(),
    };
    write_connection(&connection_path, &connection)?;
    let url = if api_only {
        format!("{origin}/api/v1/status?token={token}")
    } else {
        format!("{origin}/?token={token}")
    };
    println!(
        "Axiom Inspector {}",
        if api_only { "API" } else { "dashboard" }
    );
    println!("{url}");
    println!("Loopback only · random session token · read-only by default");
    println!(
        "Question planners: deterministic{}",
        if allow_remote_planner {
            " · Jev remote enabled (explicit per-question selection)"
        } else {
            " · Jev remote disabled"
        }
    );
    println!("Live target connection {}", connection_path.display());
    println!("Watching {}. Press Ctrl-C to stop.", workspace.display());
    if !no_open && !api_only {
        super::ui::open_application_browser(&url)?;
    }
    tokio::select! {
        result = axum::serve(listener, router) => result.context("Inspector server stopped")?,
        _ = tokio::signal::ctrl_c() => {}
    }
    let _ = fs::remove_file(&connection_path);
    Ok(())
}

fn write_connection(path: &Path, connection: &InspectorConnection) -> Result<()> {
    connection.validate()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_jcs::to_vec(connection)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)?;
    Ok(())
}

async fn index(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
) -> Response<Body> {
    if query.token.as_deref() != Some(state.token.as_ref()) {
        return response(
            StatusCode::UNAUTHORIZED,
            "text/plain; charset=utf-8",
            "invalid Inspector session token",
        );
    }
    response(
        StatusCode::OK,
        "text/html; charset=utf-8",
        inspector_dashboard::HTML,
    )
}

async fn styles() -> Response<Body> {
    response(
        StatusCode::OK,
        "text/css; charset=utf-8",
        inspector_dashboard::CSS,
    )
}

async fn script() -> Response<Body> {
    response(
        StatusCode::OK,
        "text/javascript; charset=utf-8",
        inspector_dashboard::JS,
    )
}

async fn api_status(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid Inspector session token"})),
        );
    }
    let evidence = state.evidence.read().await;
    let runtime = state.runtime.read().await;
    let generation = *state.generation.read().await;
    let updated_ms = *state.updated_ms.read().await;
    let source_error = state.source_error.read().await.clone();
    (
        StatusCode::OK,
        Json(json!({
            "format": "axiom-inspector-api/v1",
            "workspace": state.workspace.to_string_lossy(),
            "graphRevision": evidence.graph_revision,
            "source": {"generation": generation, "updatedUnixMs": updated_ms, "fresh": source_error.is_none(), "error": source_error},
            "runtime": runtime.as_ref().map(|session| json!({
                "sessionId": session.session_id,
                "capture": session.capture,
                "target": session.target,
                "graphRevision": session.graph_revision,
                "attached": true,
                "stale": session.graph_revision != evidence.graph_revision,
                "partial": !session.retention.complete,
            })).unwrap_or(json!({"attached":false})),
            "planners": {
                "deterministic": true,
                "jevRemote": state.allow_remote_planner,
                "jevModel": inspector_jev::JEV_MODEL,
            },
        })),
    )
}

async fn api_evidence(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, Json(Value::Null));
    }
    (
        StatusCode::OK,
        Json(serde_json::to_value(state.evidence.read().await.clone()).unwrap_or(Value::Null)),
    )
}

async fn api_source(
    State(state): State<ServerState>,
    Query(query): Query<SourceQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid Inspector session token"})),
        );
    }

    let reference = {
        let evidence = state.evidence.read().await;
        let Some(node) = evidence.nodes.iter().find(|node| node.id == query.node_id) else {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"semantic fact was not found"})),
            );
        };
        let Some(reference) = node.evidence.get(query.evidence_index) else {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error":"source evidence was not found"})),
            );
        };
        reference.clone()
    };

    let Some(relative) = reference.path.as_deref() else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"this evidence has no local source path"})),
        );
    };
    let requested = state.workspace.join(relative);
    let Ok(resolved) = fs::canonicalize(&requested) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"the evidenced source file is unavailable"})),
        );
    };
    if !resolved.starts_with(state.workspace.as_ref()) || !resolved.is_file() {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"the evidenced path is outside the Inspector workspace"})),
        );
    }
    let Ok(bytes) = fs::read(&resolved) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"the evidenced source file could not be read"})),
        );
    };
    if bytes.len() > 2 * 1024 * 1024 {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error":"source preview is limited to 2 MiB"})),
        );
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"binary artifact evidence has no source preview"})),
        );
    };

    let digest = hex::encode(Sha256::digest(&bytes));
    let (start, end) = reference
        .detail
        .as_deref()
        .and_then(parse_source_span)
        .unwrap_or((0, 0));
    let preview = source_preview(text, start, end);
    let language = resolved
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("text");
    (
        StatusCode::OK,
        Json(json!({
            "format":"axiom-inspector-source/v1",
            "nodeId":query.node_id,
            "path":relative,
            "absolutePath":resolved.to_string_lossy(),
            "language":language,
            "evidenceKind":reference.kind,
            "expectedSha256":reference.sha256,
            "actualSha256":digest,
            "digestMatches":reference.sha256.as_ref().map(|expected| expected == &digest),
            "startLine":preview.start_line,
            "endLine":preview.end_line,
            "startColumn":preview.start_column,
            "endColumn":preview.end_column,
            "lines":preview.lines,
        })),
    )
}

async fn api_query(
    State(state): State<ServerState>,
    Query(token): Query<TokenQuery>,
    headers: HeaderMap,
    Json(query): Json<AxiomQuery>,
) -> impl IntoResponse {
    if !authorized(&state, &headers, token.token.as_deref()) || !origin_allowed(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"request was not authorized"})),
        );
    }
    let evidence = state.evidence.read().await;
    match EvidenceIndex::new(&evidence).and_then(|index| index.query(query)) {
        Ok(result) => (
            StatusCode::OK,
            Json(serde_json::to_value(result).unwrap_or(Value::Null)),
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        ),
    }
}

async fn api_runtime(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, Json(Value::Null));
    }
    let value = state
        .runtime
        .read()
        .await
        .clone()
        .map(|session| serde_json::to_value(session).unwrap_or(Value::Null))
        .unwrap_or(Value::Null);
    (StatusCode::OK, Json(value))
}

async fn api_changes(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, Json(Value::Null));
    }
    let current = state.evidence.read().await.clone();
    let value = latest_change_report(&state.workspace, &current)
        .and_then(|report| serde_json::to_value(report).ok())
        .unwrap_or_else(|| json!({
            "format":"axiom-semantic-change/v1",
            "policy":"axiom-change-policy/v1",
            "beforeGraphRevision":null,
            "afterGraphRevision":current.graph_revision,
            "summary":{"total":0,"breaking":0,"approvalRequired":0,"warnings":0,"informational":0},
            "changes":[],"runtimeChanges":[],"baselineAvailable":false
        }));
    (StatusCode::OK, Json(value))
}

async fn api_question(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    Json(input): Json<QuestionApiRequest>,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, Json(Value::Null));
    }
    let evidence = state.evidence.read().await.clone();
    let planner = input.planner.unwrap_or(InspectorPlanner::Deterministic);
    let mut planning = None;
    let request = match (input.question, input.request) {
        (Some(question), None) if planner == InspectorPlanner::Jev => {
            if !state.allow_remote_planner {
                return (
                    StatusCode::FORBIDDEN,
                    Json(
                        json!({"error":"remote Jev planning is disabled; restart Inspector with --allow-remote-planner"}),
                    ),
                );
            }
            let receipt = match inspector_jev::plan(
                &evidence,
                &question,
                state.jev_intent_threshold,
                state.jev_selector_threshold,
            )
            .await
            {
                Ok(receipt) => receipt,
                Err(error) => {
                    return (
                        StatusCode::BAD_GATEWAY,
                        Json(json!({"error": error.to_string()})),
                    )
                }
            };
            let receipt_path = match inspector_jev::write_receipt(&state.workspace, &receipt) {
                Ok(path) => path,
                Err(error) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error":error.to_string()})),
                    )
                }
            };
            let request = receipt
                .request
                .clone()
                .or_else(|| plan_question(&question).ok());
            planning = Some(json!({
                "mode": "jev",
                "remote": true,
                "receiptFormat": receipt.format,
                "receiptPath": receipt_path,
                "provider": receipt.provider,
                "requestedModel": receipt.requested_model,
                "resolvedModel": receipt.resolved_model,
                "decision": receipt.decision,
                "thresholds": receipt.thresholds,
                "questionSha256": receipt.question_sha256,
                "candidateCount": receipt.candidate_count,
                "deterministicFallback": receipt.request.is_none() && request.is_some(),
            }));
            let Some(request) = request else {
                let answer = unsupported_answer(&evidence, receipt.decision.reason);
                let mut value = json!(answer);
                value
                    .as_object_mut()
                    .unwrap()
                    .insert("planning".into(), planning.unwrap());
                return (StatusCode::OK, Json(value));
            };
            request
        }
        (Some(question), None) => match plan_question(&question) {
            Ok(request) => request,
            Err(error) => {
                return (
                    StatusCode::OK,
                    Json(json!(unsupported_answer(&evidence, error.to_string()))),
                )
            }
        },
        (None, Some(request)) if planner == InspectorPlanner::Deterministic => request,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    json!({"error":"provide exactly one of question or request; pre-planned requests cannot select Jev"}),
                ),
            )
        }
    };
    if let Err(error) = request.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        );
    }
    let changes = latest_change_report(&state.workspace, &evidence);
    match answer_question(&evidence, request, changes.as_ref()) {
        Ok(answer) => {
            let mut value = json!(answer);
            if let Some(planning) = planning {
                value
                    .as_object_mut()
                    .expect("QuestionAnswer serializes as an object")
                    .insert("planning".into(), planning);
            }
            (StatusCode::OK, Json(value))
        }
        Err(error) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":error.to_string()})),
        ),
    }
}

fn latest_change_report(
    workspace: &Path,
    current: &ApplicationEvidence,
) -> Option<axiom_lib::application_change::SemanticChangeReport> {
    let snapshot_root = workspace.join(".axiom/inspector/snapshots");
    let mut candidates = fs::read_dir(snapshot_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let bytes = fs::read(entry.path()).ok()?;
            let evidence = ApplicationEvidence::decode(&bytes).ok()?;
            (evidence.graph_revision != current.graph_revision).then_some(evidence)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.graph_revision.cmp(&right.graph_revision));
    candidates
        .last()
        .and_then(|baseline| semantic_diff(baseline, current).ok())
}

async fn api_live(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, Json(Value::Null));
    }
    let live = state.live.read().await;
    let targets = live.targets.values().cloned().collect::<Vec<_>>();
    let evidence = state.evidence.read().await;
    let selected = targets
        .iter()
        .filter_map(|target| {
            target
                .selection
                .as_ref()
                .map(|selection| (target, selection))
        })
        .filter_map(|(target, selection)| {
            let node = evidence
                .nodes
                .iter()
                .find(|node| node.id == selection.semantic_id)?;
            let related = evidence
                .edges
                .iter()
                .filter(|edge| edge.from == node.id || edge.to == node.id)
                .cloned()
                .collect::<Vec<_>>();
            Some(json!({"target":target.handshake.target,"node":node,"relationships":related}))
        })
        .collect::<Vec<_>>();
    (
        StatusCode::OK,
        Json(json!({
            "format":"axiom-inspector-live-snapshot/v1",
            "graphRevision":evidence.graph_revision,
            "targets":targets,
            "selectedEvidence":selected,
        })),
    )
}

async fn api_live_handshake(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    Json(handshake): Json<LiveHandshake>,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid Inspector session token"})),
        );
    }
    if let Err(error) = handshake.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        );
    }
    let graph_revision = state.evidence.read().await.graph_revision.clone();
    let exact = handshake.graph_revision == graph_revision;
    let response = LiveHandshakeResponse {
        format: LIVE_HANDSHAKE_FORMAT.into(),
        accepted: exact,
        exact_graph: exact,
        application_graph_revision: graph_revision,
        message: if exact {
            "exact semantic graph attached".into()
        } else {
            "target graph differs; selection correlation is refused until the target reloads".into()
        },
    };
    let key = live_key(&handshake.target, &handshake.session_id);
    state.live.write().await.targets.insert(
        key,
        LiveTargetSnapshot {
            handshake,
            exact_graph: exact,
            paused: false,
            follow: true,
            selection: None,
            state_history: Vec::new(),
            causal_history: Vec::new(),
            last_seen_unix_ms: now_ms(),
        },
    );
    let _ = state
        .updates
        .send(json!({"type":"live-updated"}).to_string());
    (
        if exact {
            StatusCode::OK
        } else {
            StatusCode::CONFLICT
        },
        Json(json!(response)),
    )
}

async fn api_live_selection(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    Json(mut selection): Json<LiveSelection>,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid Inspector session token"})),
        );
    }
    if let Err(error) = selection.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        );
    }
    let evidence = state.evidence.read().await;
    let graph_revision = evidence.graph_revision.clone();
    if selection.graph_revision != graph_revision {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error":"selection graph revision mismatch; correlation refused",
                "partialEvidence":true,
                "targetGraphRevision":selection.graph_revision,
                "applicationGraphRevision":graph_revision,
            })),
        );
    }
    let rendered_semantic_id = selection.semantic_id.clone();
    let resolved = evidence.nodes.iter().find(|node| {
        node.id == rendered_semantic_id
            || node
                .attributes
                .get("renderedSemanticId")
                .and_then(Value::as_str)
                == Some(rendered_semantic_id.as_str())
    });
    let Some(resolved) = resolved else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"selected semantic ID is absent from the current graph"})),
        );
    };
    if resolved.id != rendered_semantic_id {
        selection.rendered_semantic_id = Some(rendered_semantic_id);
        selection.semantic_id = resolved.id.clone();
    }
    drop(evidence);
    let key = live_key(&selection.target, &selection.session_id);
    let mut live = state.live.write().await;
    let Some(target) = live.targets.get_mut(&key) else {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            Json(json!({"error":"live handshake required"})),
        );
    };
    if !target.exact_graph || target.paused {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"target is paused or not attached to the exact graph"})),
        );
    }
    target.last_seen_unix_ms = now_ms();
    target.selection = Some(selection.clone());
    let _ = state.updates.send(json!({"type":"live-selection","semanticId":selection.semantic_id,"target":selection.target}).to_string());
    (StatusCode::ACCEPTED, Json(json!({"accepted":true})))
}

async fn api_live_state(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    Json(mut event): Json<LiveStateEvent>,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid Inspector session token"})),
        );
    }
    if let Err(error) = event.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        );
    }
    let evidence = state.evidence.read().await;
    let graph_revision = evidence.graph_revision.clone();
    if event.graph_revision != graph_revision {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"state event graph mismatch","partialEvidence":true})),
        );
    }
    if let Some(resolved) = resolve_live_semantic_id(&evidence, &event.state_semantic_id) {
        event.state_semantic_id = resolved;
    }
    if let Some(resolved) = resolve_live_semantic_id(&evidence, &event.writer_semantic_id) {
        event.writer_semantic_id = resolved;
    }
    drop(evidence);
    let key = live_key(&event.target, &event.session_id);
    let mut live = state.live.write().await;
    let Some(target) = live.targets.get_mut(&key) else {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            Json(json!({"error":"live handshake required"})),
        );
    };
    target.state_history.push(event);
    if target.state_history.len() > 256 {
        target.state_history.remove(0);
    }
    target.last_seen_unix_ms = now_ms();
    let _ = state.updates.send(json!({"type":"live-state"}).to_string());
    (StatusCode::ACCEPTED, Json(json!({"accepted":true})))
}

async fn api_live_causal(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    Json(mut event): Json<LiveCausalEvent>,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"invalid Inspector session token"})),
        );
    }
    if let Err(error) = event.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        );
    }
    let evidence = state.evidence.read().await;
    let graph_revision = evidence.graph_revision.clone();
    if event.graph_revision != graph_revision {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"causal event graph mismatch","partialEvidence":true})),
        );
    }
    if let Some(resolved) = resolve_live_semantic_id(&evidence, &event.semantic_id) {
        event.semantic_id = resolved;
    }
    if let Some(parent) = event.parent_semantic_id.as_deref() {
        if let Some(resolved) = resolve_live_semantic_id(&evidence, parent) {
            event.parent_semantic_id = Some(resolved);
        }
    }
    drop(evidence);
    let key = live_key(&event.target, &event.session_id);
    let mut live = state.live.write().await;
    let Some(target) = live.targets.get_mut(&key) else {
        return (
            StatusCode::PRECONDITION_REQUIRED,
            Json(json!({"error":"live handshake required"})),
        );
    };
    target.causal_history.push(event);
    if target.causal_history.len() > 512 {
        target.causal_history.remove(0);
    }
    target.last_seen_unix_ms = now_ms();
    let _ = state
        .updates
        .send(json!({"type":"live-causal"}).to_string());
    (StatusCode::ACCEPTED, Json(json!({"accepted":true})))
}

async fn api_live_commands(
    State(state): State<ServerState>,
    Query(query): Query<LiveQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) {
        return (StatusCode::UNAUTHORIZED, Json(Value::Null));
    }
    let Some(target) = query.target else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"target is required"})),
        );
    };
    let Some(session_id) = query.session_id else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"sessionId is required"})),
        );
    };
    let after = query.after.unwrap_or(0);
    let live = state.live.read().await;
    let commands = live
        .commands
        .iter()
        .filter(|command| {
            command.target == target && command.session_id == session_id && command.id > after
        })
        .cloned()
        .collect::<Vec<_>>();
    (StatusCode::OK, Json(json!({"commands":commands})))
}

async fn api_live_command(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    Json(request): Json<CommandRequest>,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) || !origin_allowed(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"request was not authorized"})),
        );
    }
    let current = state.evidence.read().await.graph_revision.clone();
    if request.graph_revision != current {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":"command graph revision mismatch","partialEvidence":true})),
        );
    }
    let mut live = state.live.write().await;
    let key = live_key(&request.target, &request.session_id);
    let Some(target) = live.targets.get_mut(&key) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error":"live target is not connected"})),
        );
    };
    match request.kind {
        LiveCommandKind::Pause => target.paused = true,
        LiveCommandKind::Follow => {
            target.paused = false;
            target.follow = true;
        }
        LiveCommandKind::Highlight | LiveCommandKind::ClearHighlight => {}
    }
    live.next_command += 1;
    let command = LiveCommand {
        format: LIVE_COMMAND_FORMAT.into(),
        id: live.next_command,
        target: request.target,
        session_id: request.session_id,
        graph_revision: request.graph_revision,
        kind: request.kind,
        semantic_id: request.semantic_id,
    };
    if let Err(error) = command.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        );
    }
    live.commands.push(command.clone());
    if live.commands.len() > 512 {
        live.commands.remove(0);
    }
    let _ = state
        .updates
        .send(json!({"type":"live-command"}).to_string());
    (StatusCode::CREATED, Json(json!(command)))
}

fn live_key(target: &str, session_id: &str) -> String {
    format!("{target}\0{session_id}")
}

async fn api_ingest_runtime(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
    Json(session): Json<RuntimeSession>,
) -> impl IntoResponse {
    if !authorized(&state, &headers, query.token.as_deref()) || !origin_allowed(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"request was not authorized"})),
        );
    }
    let graph_revision = state.evidence.read().await.graph_revision.clone();
    if let Err(error) = session.validate() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":error.to_string()})),
        );
    }
    if session.graph_revision != graph_revision {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error":"runtime graph revision does not match the current application",
                "runtimeGraphRevision":session.graph_revision,
                "applicationGraphRevision":graph_revision,
            })),
        );
    }
    if let Err(error) = inspector_runtime::persist(&state.workspace, &session) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":error.to_string()})),
        );
    }
    *state.runtime.write().await = Some(session.clone());
    let _ = state
        .updates
        .send(json!({"type":"runtime-updated","sessionId":session.session_id}).to_string());
    (
        StatusCode::CREATED,
        Json(json!({"sessionId":session.session_id})),
    )
}

async fn api_events(
    State(state): State<ServerState>,
    Query(query): Query<TokenQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl futures::Stream<Item = std::result::Result<SseEvent, Infallible>>>, StatusCode>
{
    if !authorized(&state, &headers, query.token.as_deref()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let receiver = state.updates.subscribe();
    let stream = futures::stream::unfold(receiver, |mut receiver| async move {
        loop {
            match receiver.recv().await {
                Ok(value) => return Some((Ok(SseEvent::default().data(value)), receiver)),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    let value = json!({"type":"updates-dropped","count":skipped}).to_string();
                    return Some((Ok(SseEvent::default().data(value)), receiver));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    Ok(Sse::new(stream).keep_alive(axum::response::sse::KeepAlive::default()))
}

async fn watch_workspace(state: ServerState) {
    let mut fingerprint = workspace_fingerprint(&state.workspace).unwrap_or_default();
    let mut interval = tokio::time::interval(Duration::from_millis(750));
    loop {
        interval.tick().await;
        let next = match workspace_fingerprint(&state.workspace) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if next == fingerprint {
            continue;
        }
        fingerprint = next;
        let path = state.workspace.as_ref().clone();
        match tokio::task::spawn_blocking(move || super::inspector::load(&path)).await {
            Ok(Ok(evidence)) => {
                let revision = evidence.graph_revision.clone();
                *state.evidence.write().await = evidence;
                let _ = write_connection(
                    &state
                        .workspace
                        .join(".axiom/inspector/live-connection.json"),
                    &InspectorConnection {
                        format: LIVE_CONNECTION_FORMAT.into(),
                        endpoint: state.origin.to_string(),
                        token: state.token.to_string(),
                        graph_revision: revision.clone(),
                        process_id: std::process::id(),
                    },
                );
                *state.generation.write().await += 1;
                *state.updated_ms.write().await = now_ms();
                *state.source_error.write().await = None;
                let _ = state
                    .updates
                    .send(json!({"type":"evidence-updated","graphRevision":revision}).to_string());
            }
            Ok(Err(error)) => {
                *state.source_error.write().await = Some(error.to_string());
                let _ = state
                    .updates
                    .send(json!({"type":"diagnostic","message":error.to_string()}).to_string());
            }
            Err(_) => {}
        }
    }
}

fn workspace_fingerprint(root: &Path) -> Result<String> {
    fn visit(path: &Path, root: &Path, entries: &mut BTreeMap<String, (u64, u128)>) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if matches!(name.as_ref(), ".git" | "target" | "node_modules")
                    || path.ends_with(".axiom/inspector")
                {
                    continue;
                }
                visit(&path, root, entries)?;
            } else if relevant_source(&path) {
                let metadata = entry.metadata()?;
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                entries.insert(
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .into(),
                    (metadata.len(), modified),
                );
            }
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries)?;
    Ok(hex::encode(Sha256::digest(serde_jcs::to_vec(&entries)?)))
}

fn relevant_source(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    matches!(
        extension,
        "acore" | "axiom" | "toml" | "json" | "rs" | "lock"
    )
}

struct SourcePreview {
    start_line: usize,
    end_line: usize,
    start_column: usize,
    end_column: usize,
    lines: Vec<Value>,
}

fn parse_source_span(detail: &str) -> Option<(usize, usize)> {
    let (start, end) = detail.split_once("..")?;
    let start = start.parse::<usize>().ok()?;
    let end = end.parse::<usize>().ok()?;
    (start <= end).then_some((start, end))
}

fn source_preview(source: &str, requested_start: usize, requested_end: usize) -> SourcePreview {
    let mut start = requested_start.min(source.len());
    let mut end = requested_end.max(start).min(source.len());
    while start > 0 && !source.is_char_boundary(start) {
        start -= 1;
    }
    while end > start && !source.is_char_boundary(end) {
        end -= 1;
    }

    let start_line = source.as_bytes()[..start]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1;
    let end_offset = end.saturating_sub(1).max(start);
    let end_line = source.as_bytes()[..end_offset.min(source.len())]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
        + 1;
    let line_start = source[..start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let end_line_start = source[..end]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let start_column = source[line_start..start].chars().count() + 1;
    let end_column = source[end_line_start..end].chars().count() + 1;

    let all_lines = source.split('\n').collect::<Vec<_>>();
    let first = start_line.saturating_sub(4).max(1);
    let last = if requested_start == 0 && requested_end == 0 {
        all_lines.len().min(32)
    } else {
        (end_line + 3).min(all_lines.len())
    };
    let lines = (first..=last.max(first))
        .filter_map(|number| {
            all_lines.get(number - 1).map(|text| {
                json!({
                    "number":number,
                    "text":text,
                    "highlighted":requested_end > requested_start
                        && number >= start_line
                        && number <= end_line,
                })
            })
        })
        .collect();
    SourcePreview {
        start_line,
        end_line,
        start_column,
        end_column,
        lines,
    }
}

fn resolve_live_semantic_id(evidence: &ApplicationEvidence, incoming: &str) -> Option<String> {
    evidence
        .nodes
        .iter()
        .find(|node| {
            node.id == incoming
                || node
                    .attributes
                    .get("renderedSemanticId")
                    .and_then(Value::as_str)
                    == Some(incoming)
                || (node.kind == EvidenceNodeKind::Extension && node.label == incoming)
        })
        .map(|node| node.id.clone())
}

fn authorized(state: &ServerState, headers: &HeaderMap, query: Option<&str>) -> bool {
    query == Some(state.token.as_ref())
        || headers
            .get("x-axiom-inspector-token")
            .and_then(|value| value.to_str().ok())
            == Some(state.token.as_ref())
}

fn origin_allowed(state: &ServerState, headers: &HeaderMap) -> bool {
    headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(|origin| origin == state.origin.as_ref())
        .unwrap_or(true)
}

fn response(status: StatusCode, content_type: &str, body: &'static str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "no-store")
        .header("x-content-type-options", "nosniff")
        .header("x-frame-options", "DENY")
        .header("referrer-policy", "no-referrer")
        .header(
            "content-security-policy",
            "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; font-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        )
        .body(Body::from(body))
        .expect("valid Inspector response")
}

fn random_token() -> String {
    let entropy = format!("{}:{}", Uuid::new_v4(), Uuid::new_v4());
    hex::encode(Sha256::digest(entropy.as_bytes()))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_opaque() {
        let one = random_token();
        let two = random_token();
        assert_eq!(one.len(), 64);
        assert_ne!(one, two);
    }

    #[test]
    fn source_filter_excludes_unrelated_files() {
        assert!(relevant_source(Path::new("main.acore")));
        assert!(relevant_source(Path::new("AxiomDeps.toml")));
        assert!(!relevant_source(Path::new("photo.png")));
    }

    #[test]
    fn source_preview_converts_byte_spans_to_readable_lines() {
        let source = "page Home {\n  state total: Int = 0\n  Text(total)\n}\n";
        let start = source.find("state").unwrap();
        let end = start + "state total: Int = 0".len();
        let preview = source_preview(source, start, end);
        assert_eq!(preview.start_line, 2);
        assert_eq!(preview.end_line, 2);
        assert_eq!(preview.start_column, 3);
        assert!(preview.lines.iter().any(|line| {
            line.get("number") == Some(&json!(2)) && line.get("highlighted") == Some(&json!(true))
        }));
    }
}
