//! Mock-server tests for the recovery commands and the step-up flow.
use super::*;
use crate::commands::step_up::StepUpStore;
use axum::body::Bytes;
use axum::http::{HeaderMap, Method as HttpMethod, Uri};
use axum::response::Response;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

const RESOURCE: &str = "6f1c2a8e-0d4b-4c39-9a51-6a2f3f0b9e11";
const KEY: &str = "0b7d2f8a-61a3-4f4e-9d52-27f2b5f0c111";
const CANARY: &str = "SYNTHETIC_PRIVATE_CANARY";

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    bearer: Option<String>,
    key: Option<String>,
    body: Value,
}

#[derive(Default)]
struct Mock {
    scripts: HashMap<String, VecDeque<(u16, Value, Option<u64>)>>,
    seen: Vec<Seen>,
}

type Shared = Arc<Mutex<Mock>>;

impl Mock {
    fn script(shared: &Shared, route: &str, status: u16, body: Value) {
        Self::script_retry(shared, route, status, body, None)
    }
    fn script_retry(shared: &Shared, route: &str, status: u16, body: Value, retry: Option<u64>) {
        shared
            .lock()
            .unwrap()
            .scripts
            .entry(route.to_string())
            .or_default()
            .push_back((status, body, retry));
    }
}

fn seen(shared: &Shared, route: &str) -> Vec<Seen> {
    shared
        .lock()
        .unwrap()
        .seen
        .iter()
        .filter(|s| format!("{} {}", s.method, s.path) == route)
        .cloned()
        .collect()
}

async fn handler(
    axum::extract::State(shared): axum::extract::State<Shared>,
    method: HttpMethod,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let route = format!("{} {}", method, uri.path());
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let mut mock = shared.lock().unwrap();
    mock.seen.push(Seen {
        method: method.to_string(),
        path: uri.path().to_string(),
        bearer: header("authorization").and_then(|v| v.strip_prefix("Bearer ").map(str::to_string)),
        key: header("idempotency-key"),
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });
    // The last scripted answer for a route repeats.
    let answer = match mock.scripts.get_mut(&route) {
        Some(queue) if queue.len() > 1 => queue.pop_front(),
        Some(queue) => queue.front().cloned(),
        None => None,
    };
    let (status, body, retry) = answer.unwrap_or((404, json!({}), None));
    let mut builder = Response::builder()
        .status(status)
        .header("content-type", "application/json");
    if let Some(retry) = retry {
        builder = builder.header("retry-after", retry.to_string());
    }
    builder
        .body(axum::body::Body::from(body.to_string()))
        .unwrap()
}

struct Fixture {
    shared: Shared,
    api: Api,
    store: StepUpStore,
    _dir: tempfile::TempDir,
}

async fn fixture() -> Fixture {
    let shared: Shared = Arc::default();
    let router = axum::Router::new()
        .fallback(handler)
        .with_state(shared.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let dir = tempfile::tempdir().unwrap();
    let store = StepUpStore::at(dir.path().join("step-up.json"));
    let api = Api::new(
        &base,
        "session-access".into(),
        "session-refresh".into(),
        false,
        store.clone(),
    )
    .unwrap()
    .fast();
    Fixture {
        shared,
        api,
        store,
        _dir: dir,
    }
}

fn route(action: &str) -> String {
    format!("POST /api/v1/projects/demo/resources/{RESOURCE}/{action}")
}

fn script_step_up(shared: &Shared, token: Value) {
    Mock::script(
        shared,
        "POST /auth/device/start",
        200,
        json!({"device_code":"dc-1","user_code":"ABCD-EFGH","verification_uri":"https://example.test/device","interval":0,"expires_in":600}),
    );
    Mock::script(
        shared,
        "POST /auth/device/poll",
        200,
        json!({"error":"authorization_pending"}),
    );
    Mock::script(shared, "POST /auth/device/poll", 200, token);
}

fn step_up_token() -> Value {
    json!({"access_token":"step-up-access","expires_in":900,"scope":"resources:recovery","token_type":"Bearer"})
}

fn restore_at<'a>(at: &'a str, confirm: Option<&'a str>) -> ActionInput<'a> {
    ActionInput {
        resource: RESOURCE,
        action: "restore",
        generation: 3,
        at: Some(at),
        backup: None,
        confirm,
    }
}

const AT: &str = "2026-10-29T08:15:00Z";
const CONFIRM_AT: &str = "restore shop to 2026-10-29T08:15:00Z";

#[tokio::test]
async fn recovery_view_reads_the_route_and_renders_plain_words() {
    let f = fixture().await;
    Mock::script(
        &f.shared,
        &format!("GET /api/v1/projects/demo/resources/{RESOURCE}/recovery"),
        200,
        json!({
            "database_name": "shop\u{1b}[31m",
            "generation": 3,
            "status": "ready",
            "window": {"earliest": "2026-10-28T09:00:00Z", "latest": "2026-10-29T09:00:00Z"},
            "backups": [
                {"id": "b-1", "kind": "daily", "taken_at": "2026-10-29T02:31:00Z", "size_bytes": 2048, "verified_at": "2026-10-29T02:32:00Z"},
                {"id": "<img src=x onerror=alert(1)>", "kind": "daily", "taken_at": "2026-10-28T02:31:00Z", "verified_at": null}
            ],
            "previous": {"generation": 2, "retire_after": "2026-10-30T09:00:00Z"},
            "archive": {"state": "none"},
            "inactivity": {"last_activity_at": "2026-10-20T00:00:00Z", "archives_at": "2026-11-19T00:00:00Z"},
            "moves": [
                {"kind": "restore", "state": "activated", "from_generation": 2, "to_generation": 3, "activated_at": "2026-10-29T08:20:00Z"},
                {"kind": "unarchive", "state": "failed", "started_at": "2026-10-27T08:20:00Z", "code": "drop table; \u{7}"}
            ]
        }),
    );
    let value = view(&f.api, "demo", RESOURCE).await.unwrap();
    let text = render_recovery(&value);
    let request = &seen(
        &f.shared,
        &format!("GET /api/v1/projects/demo/resources/{RESOURCE}/recovery"),
    )[0];
    assert_eq!(request.bearer.as_deref(), Some("session-access"));
    assert!(
        text.contains("shop[31m, generation 3, status ready"),
        "{text}"
    );
    assert!(!text.chars().any(|c| c.is_control() && c != '\n'));
    assert!(text.contains("Any second from 2026-10-28T09:00:00Z to 2026-10-29T09:00:00Z"));
    assert!(text.contains("b-1  daily  2026-10-29T02:31:00Z, 2.0 KiB, verified"));
    assert!(text.contains(", not verified"));
    assert!(text.contains("generation 2) is kept until 2026-10-30T09:00:00Z"));
    assert!(text.contains("retire_previous"));
    assert!(text.contains("archives on 2026-11-19T00:00:00Z if unused"));
    assert!(text.contains("resume"));
    assert!(text.contains("generation 2 to 3  activated"));
    assert!(
        !text.contains("drop table"),
        "a non-code value is not shown as a code"
    );

    // Archived by an Owner, provider lost, nothing available.
    let text = render_recovery(&json!({
        "status": "archived",
        "failure_code": "AXRESOURCE_PROVIDER_LOST",
        "archive": {"state": "archived", "origin": "owner", "taken_at": "2026-10-01T00:00:00Z"}
    }));
    assert!(text.contains("does not resume on use"));
    assert!(text.contains("Only AxiomCore support can restore it"));
    assert!(text.contains("Not available now."));
    assert!(text.contains("None yet."));
}

#[tokio::test]
async fn restore_point_in_time_sends_the_contract_body_and_key() {
    let f = fixture().await;
    Mock::script(
        &f.shared,
        &route("restore"),
        202,
        json!({"operation_id": "op-1"}),
    );
    let outcome = run_action(
        &f.api,
        "demo",
        &restore_at(AT, Some(CONFIRM_AT)),
        Some(KEY),
        false,
        true,
    )
    .await
    .unwrap();
    let request = &seen(&f.shared, &route("restore"))[0];
    assert_eq!(
        request.body,
        json!({"generation": 3, "source": {"kind": "point_in_time", "at": AT}, "confirm": CONFIRM_AT})
    );
    assert_eq!(request.key.as_deref(), Some(KEY));
    assert_eq!(request.bearer.as_deref(), Some("session-access"));
    assert_eq!(outcome.value["operation_id"], "op-1");
    assert!(outcome.text.contains("Requested restore (operation op-1)"));
}

#[tokio::test]
async fn restore_from_backup_sends_backup_source() {
    let f = fixture().await;
    Mock::script(
        &f.shared,
        &route("restore"),
        202,
        json!({"operation_id": "op-2"}),
    );
    let input = ActionInput {
        resource: RESOURCE,
        action: "restore",
        generation: 4,
        at: None,
        backup: Some("b-1"),
        confirm: Some("restore shop to b-1"),
    };
    run_action(&f.api, "demo", &input, Some(KEY), false, false)
        .await
        .unwrap();
    assert_eq!(
        seen(&f.shared, &route("restore"))[0].body,
        json!({"generation": 4, "source": {"kind": "backup", "backup_id": "b-1"}, "confirm": "restore shop to b-1"})
    );
}

#[tokio::test]
async fn archive_resume_and_retire_send_their_bodies() {
    let f = fixture().await;
    for action in ["archive", "unarchive", "retire_previous"] {
        Mock::script(
            &f.shared,
            &route(action),
            202,
            json!({"operation_id": format!("op-{action}")}),
        );
    }
    let archive = ActionInput {
        resource: RESOURCE,
        action: "archive",
        generation: 3,
        at: None,
        backup: None,
        confirm: Some("archive shop"),
    };
    let resume = ActionInput {
        resource: RESOURCE,
        action: "unarchive",
        generation: 3,
        at: None,
        backup: None,
        confirm: None,
    };
    let retire = ActionInput {
        resource: RESOURCE,
        action: "retire_previous",
        generation: 3,
        at: None,
        backup: None,
        confirm: Some("retire previous shop"),
    };
    let outcome = run_action(&f.api, "demo", &archive, None, false, true)
        .await
        .unwrap();
    assert!(outcome.text.contains("Requested archive"));
    let outcome = run_action(&f.api, "demo", &resume, None, false, true)
        .await
        .unwrap();
    assert!(
        outcome.text.contains("Requested resume"),
        "unarchive is labelled resume"
    );
    run_action(&f.api, "demo", &retire, None, false, true)
        .await
        .unwrap();
    assert_eq!(
        seen(&f.shared, &route("archive"))[0].body,
        json!({"generation": 3, "confirm": "archive shop"})
    );
    assert_eq!(
        seen(&f.shared, &route("unarchive"))[0].body,
        json!({"generation": 3})
    );
    assert_eq!(
        seen(&f.shared, &route("retire_previous"))[0].body,
        json!({"generation": 3, "confirm": "retire previous shop"})
    );
    // Every request carries a fresh UUID Idempotency-Key.
    for action in ["archive", "unarchive", "retire_previous"] {
        let key = seen(&f.shared, &route(action))[0].key.clone().unwrap();
        uuid::Uuid::parse_str(&key).unwrap();
    }
}

#[test]
fn local_validation_explains_consequences_and_refuses_bad_flags() {
    let error = build_body(&restore_at(AT, None)).unwrap_err().to_string();
    assert!(error.contains("Application data returns to 2026-10-29T08:15:00Z"));
    assert!(error.contains("Users, passwords and sessions stay as they are now"));
    assert!(error.contains("kept 24 hours"));
    assert!(error.contains("--confirm \"restore <database name> to 2026-10-29T08:15:00Z\""));

    let error = build_body(&restore_at(
        AT,
        Some("restore shop to 2026-10-28T08:15:00Z"),
    ))
    .unwrap_err();
    assert!(error.to_string().contains("does not match"));
    assert!(build_body(&restore_at(AT, Some("restore  to 2026-10-29T08:15:00Z"))).is_err());
    assert!(build_body(&restore_at("yesterday", Some("restore shop to yesterday"))).is_err());
    assert!(build_body(&restore_at(
        "2999-01-01T00:00:00Z",
        Some("restore shop to 2999-01-01T00:00:00Z")
    ))
    .unwrap_err()
    .to_string()
    .contains("in the past"));

    let both = ActionInput {
        resource: RESOURCE,
        action: "restore",
        generation: 3,
        at: Some(AT),
        backup: Some("b-1"),
        confirm: Some(CONFIRM_AT),
    };
    assert!(build_body(&both)
        .unwrap_err()
        .to_string()
        .contains("exactly one"));
    let neither = ActionInput {
        at: None,
        backup: None,
        ..both
    };
    assert!(build_body(&neither).is_err());
    let bad_backup = ActionInput {
        resource: RESOURCE,
        action: "restore",
        generation: 3,
        at: None,
        backup: Some("../b"),
        confirm: Some("restore shop to ../b"),
    };
    assert!(build_body(&bad_backup).is_err());

    let archive = ActionInput {
        resource: RESOURCE,
        action: "archive",
        generation: 3,
        at: None,
        backup: None,
        confirm: None,
    };
    let error = build_body(&archive).unwrap_err().to_string();
    assert!(error.contains("deleted from the hosting provider"));
    assert!(error.contains("does not resume on use"));
    assert!(build_body(&ActionInput {
        confirm: Some("archive"),
        ..archive
    })
    .is_err());
    assert!(build_body(&ActionInput {
        at: Some(AT),
        confirm: Some("archive shop"),
        ..archive
    })
    .is_err());

    let retire = ActionInput {
        resource: RESOURCE,
        action: "retire_previous",
        generation: 3,
        at: None,
        backup: None,
        confirm: None,
    };
    assert!(build_body(&retire)
        .unwrap_err()
        .to_string()
        .contains("Support can no longer recover it"));

    let resume = ActionInput {
        resource: RESOURCE,
        action: "unarchive",
        generation: 3,
        at: None,
        backup: None,
        confirm: Some("x"),
    };
    assert!(build_body(&resume).is_err());
    assert!(build_body(&ActionInput {
        generation: 0,
        confirm: None,
        ..resume
    })
    .is_err());
    assert!(build_body(&ActionInput {
        resource: "not-a-uuid",
        confirm: None,
        ..resume
    })
    .is_err());
    assert!(build_body(&ActionInput {
        confirm: Some("archive shop\u{1b}"),
        action: "archive",
        ..resume
    })
    .is_err());
}

#[tokio::test]
async fn reauth_runs_step_up_and_retries_with_the_same_key() {
    let f = fixture().await;
    Mock::script(
        &f.shared,
        &route("restore"),
        403,
        json!({"code": "AXRESOURCE_REAUTH_REQUIRED"}),
    );
    Mock::script(
        &f.shared,
        &route("restore"),
        202,
        json!({"operation_id": "op-3"}),
    );
    script_step_up(&f.shared, step_up_token());

    run_action(
        &f.api,
        "demo",
        &restore_at(AT, Some(CONFIRM_AT)),
        Some(KEY),
        false,
        true,
    )
    .await
    .unwrap();

    let start = &seen(&f.shared, "POST /auth/device/start")[0];
    assert_eq!(start.body, json!({"scope": "resources:recovery"}));
    assert_eq!(start.bearer, None, "the device flow sends no session token");
    assert_eq!(seen(&f.shared, "POST /auth/device/poll").len(), 2);
    let restores = seen(&f.shared, &route("restore"));
    assert_eq!(restores.len(), 2);
    assert_eq!(restores[0].bearer.as_deref(), Some("session-access"));
    assert_eq!(restores[1].bearer.as_deref(), Some("step-up-access"));
    assert_eq!(restores[0].key, restores[1].key);
    assert_eq!(restores[0].body, restores[1].body);

    // Stored privately, no refresh token, never the session.
    let raw = std::fs::read_to_string(f.store.path()).unwrap();
    assert!(raw.contains("step-up-access"));
    assert!(!raw.contains("refresh"));
    assert!(!raw.contains("session-access"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(f.store.path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let stored: Value = serde_json::from_str(&raw).unwrap();
    let left = stored["expires_at"].as_u64().unwrap()
        - std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
    assert!(left <= 900 && left > 800);
}

#[tokio::test]
async fn stored_step_up_token_is_used_and_an_expired_one_is_not() {
    let f = fixture().await;
    Mock::script(
        &f.shared,
        &route("archive"),
        202,
        json!({"operation_id": "op-4"}),
    );
    let archive = ActionInput {
        resource: RESOURCE,
        action: "archive",
        generation: 3,
        at: None,
        backup: None,
        confirm: Some("archive shop"),
    };
    f.store.save("step-up-stored", 3600).unwrap();
    run_action(&f.api, "demo", &archive, None, false, false)
        .await
        .unwrap();
    assert_eq!(
        seen(&f.shared, &route("archive"))[0].bearer.as_deref(),
        Some("step-up-stored")
    );
    let stored: Value = serde_json::from_slice(&std::fs::read(f.store.path()).unwrap()).unwrap();
    assert!(
        stored["expires_at"].as_u64().unwrap()
            <= std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 900,
        "lifetime clamped to 15 minutes"
    );

    // Within the 30-second margin: not used, file removed, session used.
    f.store.save("step-up-stale", 10).unwrap();
    run_action(&f.api, "demo", &archive, None, false, false)
        .await
        .unwrap();
    assert_eq!(
        seen(&f.shared, &route("archive"))[1].bearer.as_deref(),
        Some("session-access")
    );
    assert!(!f.store.path().exists());
}

#[tokio::test]
async fn a_refused_step_up_token_steps_up_again_once() {
    let f = fixture().await;
    f.store.save("step-up-revoked", 900).unwrap();
    Mock::script(&f.shared, &route("retire_previous"), 401, json!({}));
    Mock::script(
        &f.shared,
        &route("retire_previous"),
        202,
        json!({"operation_id": "op-5"}),
    );
    script_step_up(&f.shared, step_up_token());
    let retire = ActionInput {
        resource: RESOURCE,
        action: "retire_previous",
        generation: 3,
        at: None,
        backup: None,
        confirm: Some("retire previous shop"),
    };
    run_action(&f.api, "demo", &retire, None, false, false)
        .await
        .unwrap();
    let requests = seen(&f.shared, &route("retire_previous"));
    assert_eq!(requests[0].bearer.as_deref(), Some("step-up-revoked"));
    assert_eq!(requests[1].bearer.as_deref(), Some("step-up-access"));
    assert!(
        seen(&f.shared, "POST /auth/token/refresh").is_empty(),
        "a step-up token is never refreshed"
    );
}

#[tokio::test]
async fn reauth_after_step_up_fails_with_the_code() {
    let f = fixture().await;
    Mock::script(
        &f.shared,
        &route("restore"),
        403,
        json!({"code": "AXRESOURCE_REAUTH_REQUIRED", "message": CANARY}),
    );
    script_step_up(&f.shared, step_up_token());
    let error = run_action(
        &f.api,
        "demo",
        &restore_at(AT, Some(CONFIRM_AT)),
        None,
        false,
        false,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("AXRESOURCE_REAUTH_REQUIRED:"), "{error}");
    assert!(error.contains("axiom login --step-up"));
    assert!(!error.contains(CANARY));
    assert_eq!(
        seen(&f.shared, &route("restore")).len(),
        2,
        "one step-up, one retry"
    );
    assert_eq!(seen(&f.shared, "POST /auth/device/start").len(), 1);
}

#[tokio::test]
async fn step_up_refuses_an_ordinary_session() {
    for token in [
        json!({"access_token": "a", "refresh_token": "r", "expires_in": 3600}),
        json!({"access_token": "a", "expires_in": 900, "scope": "secrets:development"}),
        json!({"access_token": "", "expires_in": 900}),
    ] {
        let f = fixture().await;
        script_step_up(&f.shared, token);
        let base = f.api.base.clone();
        let error = step_up(&base, &f.store, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("did not return a step-up token"), "{error}");
        assert!(!f.store.path().exists(), "nothing stored");
    }
    // Declined and expired codes.
    for (code, expected) in [
        ("access_denied", "declined"),
        ("expired_token", "expired"),
        (CANARY, "refused"),
    ] {
        let f = fixture().await;
        script_step_up(&f.shared, json!({"error": code}));
        let base = f.api.base.clone();
        let error = step_up(&base, &f.store, Duration::ZERO)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains(CANARY));
    }
}

#[tokio::test]
async fn wait_follows_the_operation_to_success_or_failure() {
    let f = fixture().await;
    Mock::script(
        &f.shared,
        &route("restore"),
        202,
        json!({"operation_id": "op-6"}),
    );
    let summary = "GET /api/v1/projects/demo/resources";
    Mock::script(
        &f.shared,
        summary,
        200,
        json!({"operations": [{"id": "op-6", "status": "running"}]}),
    );
    Mock::script(
        &f.shared,
        summary,
        200,
        json!({"operations": [{"id": "op-6", "status": "succeeded", "action": "restore"}]}),
    );
    let outcome = run_action(
        &f.api,
        "demo",
        &restore_at(AT, Some(CONFIRM_AT)),
        None,
        true,
        true,
    )
    .await
    .unwrap();
    assert!(outcome.text.starts_with("Restore complete"));
    assert_eq!(outcome.value["operation"]["status"], "succeeded");
    assert_eq!(seen(&f.shared, summary).len(), 2);

    let f = fixture().await;
    Mock::script(
        &f.shared,
        &route("unarchive"),
        202,
        json!({"operation_id": "op-7"}),
    );
    Mock::script(
        &f.shared,
        summary,
        200,
        json!({"operations": [{"id": "op-7", "status": "failed", "failure_code": "AXRESOURCE_HEAD_UNKNOWN"}]}),
    );
    let resume = ActionInput {
        resource: RESOURCE,
        action: "unarchive",
        generation: 3,
        at: None,
        backup: None,
        confirm: None,
    };
    let error = run_action(&f.api, "demo", &resume, None, true, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("The resume did not complete"), "{error}");
    assert!(error.contains("AXRESOURCE_HEAD_UNKNOWN: AxiomCore has no record of the schema"));

    let f = fixture().await;
    Mock::script(
        &f.shared,
        &route("archive"),
        202,
        json!({"operation_id": "op-8"}),
    );
    Mock::script(
        &f.shared,
        summary,
        200,
        json!({"operations": [{"id": "op-8", "status": "failed", "failure_code": format!("{CANARY} x")}]}),
    );
    let archive = ActionInput {
        resource: RESOURCE,
        action: "archive",
        generation: 3,
        at: None,
        backup: None,
        confirm: Some("archive shop"),
    };
    let error = run_action(&f.api, "demo", &archive, None, true, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains(CANARY));
}

#[tokio::test]
async fn every_contract_code_maps_to_plain_words_without_echo() {
    let cases: &[(&str, u16, Option<u64>, Value, &str)] = &[
        ("AXRESOURCE_MOVING", 503, None, json!({}), "switching"),
        (
            "AXRESOURCE_ARCHIVED",
            409,
            None,
            json!({"resumable": true}),
            "resumes on next use",
        ),
        (
            "AXRESOURCE_ARCHIVED",
            409,
            None,
            json!({"resumable": false}),
            "does not resume on use",
        ),
        (
            "AXRESOURCE_RESUMING",
            503,
            Some(30),
            json!({}),
            "Retry in 30 seconds",
        ),
        (
            "AXRESOURCE_POINT_UNAVAILABLE",
            422,
            None,
            json!({}),
            "outside the restore window",
        ),
        (
            "AXRESOURCE_CAPACITY",
            503,
            None,
            json!({}),
            "daily restore limit",
        ),
        ("AXRESOURCE_RESIDENCY", 503, None, json!({}), "region"),
        (
            "AXRESOURCE_HEAD_UNKNOWN",
            422,
            None,
            json!({}),
            "no record of the schema",
        ),
        (
            "AXRESOURCE_ARTIFACT_REFUSED",
            422,
            None,
            json!({}),
            "does not accept",
        ),
        (
            "AXRESOURCE_PREVIOUS_RETAINED",
            409,
            None,
            json!({}),
            "retire_previous",
        ),
        (
            "AXRESOURCE_PROVIDER_LOST",
            409,
            None,
            json!({}),
            "contact support",
        ),
        (
            "AUTH_RECONCILING",
            503,
            Some(45),
            json!({}),
            "Retry in 45 seconds",
        ),
        (
            "AXRESOURCE_DENIED",
            403,
            None,
            json!({}),
            "Only a project Owner",
        ),
        (
            "AXRESOURCE_CONFLICT",
            409,
            None,
            json!({}),
            "current generation",
        ),
    ];
    for (code, status, retry, extra, expected) in cases {
        let f = fixture().await;
        let mut body = json!({"code": code, "message": CANARY});
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        Mock::script_retry(&f.shared, &route("restore"), *status, body, *retry);
        let error = run_action(
            &f.api,
            "demo",
            &restore_at(AT, Some(CONFIRM_AT)),
            None,
            false,
            false,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(error.starts_with(&format!("{code}: ")), "{error}");
        assert!(error.contains(expected), "{code}: {error}");
        assert!(error.contains(&format!("(HTTP {status})")));
        assert!(!error.contains(CANARY));
        assert!(
            seen(&f.shared, "POST /auth/device/start").is_empty(),
            "{code} does not step up"
        );
    }
    // REAUTH has its own test; an unknown code is never echoed.
    let f = fixture().await;
    Mock::script(&f.shared, &route("restore"), 429, json!({"code": CANARY}));
    let error = run_action(
        &f.api,
        "demo",
        &restore_at(AT, Some(CONFIRM_AT)),
        None,
        false,
        false,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.starts_with("AXRESOURCE_CLOUD"));
    assert!(!error.contains(CANARY));
    assert!(code_message("AXRESOURCE_REAUTH_REQUIRED", None, None)
        .unwrap()
        .contains("--step-up"));
}

#[tokio::test]
async fn reads_refresh_an_expired_session_once() {
    let f = fixture().await;
    let path = format!("GET /api/v1/projects/demo/resources/{RESOURCE}/recovery");
    Mock::script(&f.shared, &path, 401, json!({}));
    Mock::script(&f.shared, &path, 200, json!({"status": "ready"}));
    Mock::script(
        &f.shared,
        "POST /auth/token/refresh",
        200,
        json!({"access_token": "session-access-2", "refresh_token": "session-refresh-2", "expires_in": 3600}),
    );
    view(&f.api, "demo", RESOURCE).await.unwrap();
    let requests = seen(&f.shared, &path);
    assert_eq!(requests[1].bearer.as_deref(), Some("session-access-2"));
    assert_eq!(
        seen(&f.shared, "POST /auth/token/refresh")[0].body["refresh_token"],
        "session-refresh"
    );
    assert!(view(&f.api, "demo", "nope").await.is_err());
}

#[test]
fn cli_parses_the_new_flags() {
    use clap::Parser;
    #[derive(Parser)]
    struct Harness {
        #[command(subcommand)]
        action: crate::commands::resources::Action,
    }
    let parsed = Harness::try_parse_from([
        "x",
        "request",
        "--resource",
        RESOURCE,
        "--action",
        "restore",
        "--generation",
        "3",
        "--at",
        AT,
        "--confirm",
        CONFIRM_AT,
        "--wait",
        "--json",
    ])
    .unwrap();
    assert!(matches!(
        parsed.action,
        crate::commands::resources::Action::Request {
            json: true,
            wait: true,
            ..
        }
    ));
    assert!(Harness::try_parse_from([
        "x",
        "request",
        "--resource",
        RESOURCE,
        "--action",
        "restore",
        "--generation",
        "3",
        "--at",
        AT,
        "--backup",
        "b-1",
    ])
    .is_err());
    for action in ["archive", "unarchive", "retire_previous"] {
        Harness::try_parse_from([
            "x",
            "request",
            "--resource",
            RESOURCE,
            "--action",
            action,
            "--generation",
            "1",
        ])
        .unwrap();
    }
    assert!(Harness::try_parse_from([
        "x",
        "request",
        "--resource",
        RESOURCE,
        "--action",
        "dr_restore",
        "--generation",
        "1"
    ])
    .is_err());
    let parsed =
        Harness::try_parse_from(["x", "recovery", "--resource", RESOURCE, "--json"]).unwrap();
    assert!(matches!(
        parsed.action,
        crate::commands::resources::Action::Recovery { json: true, .. }
    ));
}
