use super::*;
use crate::commands::cloud_mock::Mock;

fn api(base: &str, dir: &tempfile::TempDir) -> Api {
    Api::new(
        base,
        "session-token".into(),
        "refresh-token".into(),
        false,
        StepUpStore::at(dir.path().join("step-up.json")),
    )
    .unwrap()
    .fast()
}

#[tokio::test]
async fn data_preserves_method_path_body_query_and_key() {
    let mock = Mock::default();
    let route = "POST /api/v1/projects/demo/data/environments/env/sql/write/execute";
    mock.script(route, 200, json!({"receipt":{"state":"committed"}}));
    let base = mock.start().await;
    let dir = tempfile::tempdir().unwrap();
    let body = json!({"sql":"delete from items", "params":[], "confirmed_affected":2, "confirmed_total":null, "head":null});
    api(&base, &dir)
        .data_request(
            &Method::POST,
            &[
                "projects",
                "demo",
                "data",
                "environments",
                "env",
                "sql",
                "write",
                "execute",
            ],
            Some(&body),
            Some("operation-key"),
            &[("before", "cursor value".into())],
        )
        .await
        .unwrap();
    let seen = mock.seen(route);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, body);
    assert_eq!(seen[0].key.as_deref(), Some("operation-key"));
    assert_eq!(seen[0].bearer.as_deref(), Some("session-token"));
    assert_eq!(seen[0].query.as_deref(), Some("before=cursor+value"));
}

#[tokio::test]
async fn data_forbidden_does_not_attempt_step_up_and_errors_are_redacted() {
    let mock = Mock::default();
    let route = "POST /api/v1/projects/demo/data/environments/env/tables/items/rows:write";
    mock.script(
        route,
        403,
        json!({"error":{"code":"AXDATA_FORBIDDEN","message":"PRIVATE_SQL_CANARY"}}),
    );
    let base = mock.start().await;
    let dir = tempfile::tempdir().unwrap();
    let error = api(&base, &dir)
        .data_request(
            &Method::POST,
            &[
                "projects",
                "demo",
                "data",
                "environments",
                "env",
                "tables",
                "items",
                "rows:write",
            ],
            Some(&json!({"operations":[]})),
            Some("key"),
            &[],
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("AXDATA_FORBIDDEN"));
    assert!(!error.contains("PRIVATE_SQL_CANARY"));
    assert_eq!(mock.seen(route).len(), 1);
    assert!(mock.seen("POST /auth/device/start").is_empty());
}

#[tokio::test]
async fn data_pending_retries_are_bounded_and_preserve_body_and_key() {
    let mock = Mock::default();
    let route = "POST /api/v1/data";
    mock.script(route, 409, json!({"error":{"code":"AXDATA_WRITE_PENDING"}}));
    let base = mock.start().await;
    let dir = tempfile::tempdir().unwrap();
    let error = api(&base, &dir)
        .data_request(&Method::POST, &["data"], Some(&json!({})), Some("key"), &[])
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("AXDATA_WRITE_PENDING"));
    assert_eq!(mock.seen(route).len(), 4);
    for seen in mock.seen(route) {
        assert_eq!(seen.body, json!({}));
        assert_eq!(seen.key.as_deref(), Some("key"));
    }
}

#[tokio::test]
async fn data_refresh_replays_exact_body_and_key() {
    let mock = Mock::default();
    mock.script("POST /api/v1/data", 401, json!({}));
    mock.script("POST /api/v1/data", 200, json!({"ok":true}));
    mock.script("POST /auth/token/refresh", 200, json!({"access_token":"new-access", "refresh_token":"new-refresh", "expires_in":3600,"token_type":"Bearer"}));
    let base = mock.start().await;
    let dir = tempfile::tempdir().unwrap();
    let body = json!({"operations":[{"op":"insert","values":{"id":{"t":"integer","v":1}}}]});
    api(&base, &dir)
        .data_request(&Method::POST, &["data"], Some(&body), Some("key"), &[])
        .await
        .unwrap();
    let seen = mock.seen("POST /api/v1/data");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body, seen[1].body);
    assert_eq!(seen[0].key, seen[1].key);
    assert_eq!(seen[1].bearer.as_deref(), Some("new-access"));
}

async fn csv_server(text: &'static str, content_type: &'static str) -> String {
    let router = axum::Router::new().fallback(move |headers: axum::http::HeaderMap| async move {
        assert_eq!(headers["accept"], "text/csv");
        assert_eq!(headers["authorization"], "Bearer session-token");
        ([(axum::http::header::CONTENT_TYPE, content_type)], text)
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    base
}

#[tokio::test]
async fn export_validates_content_type_and_final_marker() {
    let dir = tempfile::tempdir().unwrap();
    for (text, content_type, success) in [
        ("id\r\n1\r\n# complete 1 rows\r\n", "text/csv", true),
        (
            "id\n1\n# truncated 1 rows\n",
            "text/csv; charset=utf-8",
            true,
        ),
        ("id\n1\n", "text/csv", false),
        ("id\n1\n# complete 2 rows\n", "text/csv", false),
        ("id\n\"unfinished\n# complete 1 rows\n", "text/csv", false),
        ("id\n1\n# complete 1 rows\n", "text/plain", false),
    ] {
        let base = csv_server(text, content_type).await;
        let result = api(&base, &dir)
            .data_export(&["data"], &json!({"filters":[]}))
            .await;
        assert_eq!(result.is_ok(), success);
        if success {
            assert_eq!(result.unwrap(), text);
        }
    }
}

#[test]
fn data_error_code_cannot_inject_arbitrary_server_text() {
    for code in [
        "AXDATA_VALID\nPRIVATE_SQL",
        "AXDATA_",
        "private_engine_message",
    ] {
        let error = data_result(Reply {
            status: 400,
            body: json!({"error":{"code":code}}),
            retry_after: None, text: None,
        })
        .unwrap_err()
        .to_string();
        assert!(!error.contains("PRIVATE_SQL"));
        assert!(error.starts_with("AXDATA_REQUEST_FAILED"));
    }
}

#[tokio::test]
async fn data_explicit_step_up_challenge_reuses_body_and_key() {
    let mock = Mock::default();
    mock.script(
        "POST /api/v1/data",
        403,
        json!({"code":"AXRESOURCE_REAUTH_REQUIRED"}),
    );
    mock.script("POST /api/v1/data", 200, json!({"ok":true}));
    mock.script("POST /auth/device/start", 200, json!({"device_code":"device", "user_code":"TEST", "verification_uri":"https://example.invalid/approve", "interval":0, "expires_in":60}));
    mock.script(
        "POST /auth/device/poll",
        200,
        json!({"access_token":"step-up-token", "expires_in":900, "scope":"resources:recovery"}),
    );
    let base = mock.start().await;
    let dir = tempfile::tempdir().unwrap();
    let body = json!({"operations": [{"op":"delete","key":{"id":{"t":"integer","v":1}},"original":{"id":{"t":"integer","v":1}}}]});
    api(&base, &dir)
        .data_request(&Method::POST, &["data"], Some(&body), Some("key"), &[])
        .await
        .unwrap();
    let seen = mock.seen("POST /api/v1/data");
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body, seen[1].body);
    assert_eq!(seen[0].key, seen[1].key);
    assert_eq!(seen[1].bearer.as_deref(), Some("step-up-token"));
    assert!(mock.seen("POST /auth/token/refresh").is_empty());
}

#[tokio::test]
async fn browser_only_refusals_explain_dashboard_and_preserve_role_refusals() {
    let mock = Mock::default();
    let base = mock.start().await;
    let dir = tempfile::tempdir().unwrap();
    let api = api(&base, &dir).with_data_environment("staging");
    for (method, suffix) in [
        (Method::POST, "sql/write/preview"),
        (Method::POST, "sql/write/execute"),
        (Method::POST, "tables/items/rows:write"),
        (Method::POST, "tables/items/export"),
        (Method::POST, "capture/preview"),
        (Method::POST, "capture"),
        (Method::PUT, "capture/settings"),
        (Method::GET, "overview"),
        (Method::POST, "sql/read"),
        (Method::GET, "capture/settings"),
    ] {
        let route = format!("{method} /api/v1/projects/demo/data/environments/env/{suffix}");
        let mut segments = vec!["projects", "demo", "data", "environments", "env"];
        segments.extend(suffix.split('/'));
        for reason in [None, Some("browser_session"), Some("role")] {
            let mut body =
                json!({"error":{"code":"AXDATA_FORBIDDEN", "message":"PRIVATE_SQL_CANARY"}});
            if let Some(reason) = reason {
                body["error"]["reason"] = json!(reason);
            }
            mock.script(&route, 403, body);
        }
        for reason in [None, Some("browser_session"), Some("role")] {
            let error = if suffix.ends_with("/export") {
                api.data_export(&segments, &json!({})).await.unwrap_err()
            } else {
                api.data_request(&method, &segments, None, None, &[])
                    .await
                    .unwrap_err()
            }
            .to_string();
            let expected = reason == Some("browser_session")
                || (reason.is_none()
                    && (method == Method::PUT || (method == Method::POST && suffix != "sql/read")));
            assert_eq!(
                error.contains("dashboard browser session"),
                expected,
                "{route}: {error}"
            );
            if expected {
                assert!(error.contains(&format!("{base}/dashboard")));
                assert!(error.contains("project demo, environment staging"));
            } else {
                assert!(error.contains("data request refused (HTTP 403)"));
            }
            assert!(!error.contains("PRIVATE_SQL_CANARY"));
        }
        assert_eq!(mock.seen(&route).len(), 3);
    }
    assert!(mock.seen("POST /auth/device/start").is_empty());
}

#[test]
fn browser_refusal_uses_production_dashboard_origin() {
    let reply = Reply { status: 403, body: json!({"code":"AXDATA_FORBIDDEN"}), retry_after: None, text: None };
    let error = data_route_result(reply, &Method::POST,
        &["projects", "demo", "data", "environments", "uuid", "capture"],
        axiom_cloud::AXIOM_CLOUD_URL, Some("staging")).unwrap_err().to_string();
    assert!(error.contains("https://app.axiomcore.dev/dashboard"));
    assert!(error.contains("environment staging"));
    assert!(!error.contains("uuid"));
}
