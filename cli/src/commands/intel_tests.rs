use super::*;
use crate::commands::cloud_mock::Mock;
use crate::commands::intel_routes::tests::{cases, ParserHarness};
use clap::Parser;
use serde_json::{json, Value};

fn project_fixture() -> Value {
    json!([{"id":"project-id", "name":"Demo project", "slug":"demo", "visibility":"private", "created_at":"2026-11-13", "updated_at":"2026-11-13"}])
}

#[tokio::test]
async fn all_36_json_commands_send_dashboard_method_path_and_body() {
    for (argv, method, path, mut body) in cases() {
        if path == "/api/v1/activity/stream" {
            continue;
        }
        let mock = Mock::default();
        let path = path.replace("{project}", "demo");
        let route = format!("{method} {path}");
        mock.script("GET /api/v1/projects", 200, project_fixture());
        if path.ends_with("/settings") && method == "PATCH" {
            mock.script("GET /api/v1/projects/demo", 200, json!({"name":"Demo", "description":"Keep", "default_branch":"main", "visibility":"private"}));
            body = Some(
                json!({"name":"example", "description":"Keep", "default_branch":"main", "visibility":"private"}),
            );
        }
        if path.ends_with("/alert-policies/item") && method == "PUT" {
            mock.script("GET /api/v1/projects/demo/alert-policies", 200, json!([{"kind":"item", "enabled":true, "window_seconds":300, "threshold":1.0, "minimum_samples":10, "severity":"warning"}]));
            body = Some(
                json!({"enabled":true, "window_seconds":300, "threshold":5.0, "minimum_samples":10, "severity":"warning"}),
            );
        }
        mock.script(&route, 200, json!({"ok":true}));
        let base = mock.start().await;
        let client =
            CloudClient::for_loopback(&base, "test-bearer".into(), "test-refresh".into()).unwrap();
        let args = IntelArgs {
            project: Some("demo".into()),
            environment: None,
            json: true,
            yes: true,
            show_secrets: false,
            query: vec![],
            command: ParserHarness::try_parse_from(argv.clone()).unwrap().command,
        };
        execute(&client, &args)
            .await
            .unwrap_or_else(|error| panic!("{argv:?}: {error}"));
        let seen = mock.seen(&route);
        assert_eq!(seen.len(), 1, "{argv:?}");
        assert_eq!(seen[0].body, body.unwrap_or(Value::Null), "{argv:?}");
        assert_eq!(seen[0].bearer.as_deref(), Some("test-bearer"), "{argv:?}");
    }
}

#[tokio::test]
async fn project_name_and_id_resolve_to_slug() {
    for project in ["Demo project", "project-id"] {
        let mock = Mock::default();
        mock.script("GET /api/v1/projects", 200, project_fixture());
        mock.script("GET /api/v1/projects/demo", 200, json!({"id":"project-id"}));
        let base = mock.start().await;
        let client =
            CloudClient::for_loopback(&base, "test-bearer".into(), "test-refresh".into()).unwrap();
        let args = IntelArgs {
            project: Some(project.into()),
            environment: None,
            json: true,
            yes: true,
            show_secrets: false,
            query: vec![],
            command: ParserHarness::try_parse_from(["intel", "project", "get"])
                .unwrap()
                .command,
        };
        execute(&client, &args).await.unwrap();
        assert_eq!(mock.seen("GET /api/v1/projects/demo").len(), 1);
    }
}

#[test]
fn credentials_are_redacted_recursively() {
    let mut value = json!({"source":{"name":"test","dsn":"secret-value"},"tokens":["private"]});
    redact(&mut value);
    assert_eq!(value["source"]["name"], "test");
    assert!(!value.to_string().contains("secret-value"));
    assert!(!value.to_string().contains("private"));
}

#[tokio::test]
async fn destructive_command_requires_confirmation_before_network() {
    let mock = Mock::default();
    let base = mock.start().await;
    let client = CloudClient::for_loopback(&base, "test".into(), "refresh".into()).unwrap();
    let args = IntelArgs {
        project: Some("demo".into()),
        environment: None,
        json: true,
        yes: false,
        show_secrets: false,
        query: vec![],
        command: ParserHarness::try_parse_from([
            "test",
            "observability",
            "sources",
            "revoke",
            "source-1",
        ])
        .unwrap()
        .command,
    };
    assert!(execute(&client, &args)
        .await
        .unwrap_err()
        .to_string()
        .contains("--yes"));
    assert!(mock.seen("GET /api/v1/projects").is_empty());
}

#[tokio::test]
async fn activity_stream_handles_chunked_sse_and_follow() {
    use axum::{body::Body, response::Response, routing::get};
    let router = axum::Router::new().route(
        "/stream",
        get(|| async {
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from(
                    ": heartbeat\n\ndata: {\"event\":1}\n\ndata: {\"event\":2}\n\n",
                ))
                .unwrap()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/stream", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    for follow in [false, true] {
        stream(reqwest::get(&url).await.unwrap(), true, follow)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn activity_command_uses_authenticated_sse_endpoint() {
    use axum::{body::Body, http::HeaderMap, response::Response, routing::get};
    let router = axum::Router::new().route(
        "/api/v1/activity/stream",
        get(|headers: HeaderMap| async move {
            assert_eq!(headers["authorization"], "Bearer test");
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from("data: {\"type\":\"ready\"}\r\n\r\n"))
                .unwrap()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = CloudClient::for_loopback(&url, "test".into(), "refresh".into()).unwrap();
    let args = IntelArgs {
        project: None,
        environment: None,
        json: true,
        yes: false,
        show_secrets: false,
        query: vec![],
        command: ParserHarness::try_parse_from(["intel", "activity", "stream"])
            .unwrap()
            .command,
    };
    execute(&client, &args).await.unwrap();
}

#[test]
fn sse_boundaries_support_split_crlf_frames() {
    assert_eq!(frame_end(b"data: {}\r\n\r"), None);
    assert_eq!(frame_end(b"data: {}\r\n\r\n"), Some((8, 4)));
    assert_eq!(frame_end(b": heartbeat\n\n"), Some((11, 2)));
}

#[tokio::test]
async fn environment_names_and_ids_validate_and_query_filters_reach_api() {
    for selected in ["dev", "env-id"] {
        let mock = Mock::default();
        mock.script("GET /api/v1/projects", 200, project_fixture());
        mock.script(
            "GET /api/v1/projects/demo/configuration-environments",
            200,
            json!([{"id":"env-id","name":"dev"}]),
        );
        mock.script("GET /api/v1/projects/demo/logs", 200, json!([]));
        let base = mock.start().await;
        let client = CloudClient::for_loopback(&base, "test".into(), "refresh".into()).unwrap();
        let args = IntelArgs {
            project: Some("demo".into()),
            environment: Some(selected.into()),
            json: true,
            yes: false,
            show_secrets: false,
            query: vec![("route".into(), "/hello world&limit=0".into())],
            command: ParserHarness::try_parse_from(["intel", "logs", "list"])
                .unwrap()
                .command,
        };
        execute(&client, &args).await.unwrap();
        let seen = mock.seen("GET /api/v1/projects/demo/logs");
        let query: Vec<_> =
            url::form_urlencoded::parse(seen[0].query.as_ref().unwrap().as_bytes()).collect();
        assert_eq!(query, vec![("route".into(), "/hello world&limit=0".into())]);
    }
}

#[tokio::test]
async fn logs_follow_polls_the_same_endpoint() {
    let mock = Mock::default();
    mock.script("GET /api/v1/projects", 200, project_fixture());
    mock.script("GET /api/v1/projects/demo/logs", 200, json!([]));
    let base = mock.start().await;
    let client = CloudClient::for_loopback(&base, "test".into(), "refresh".into()).unwrap();
    let args = IntelArgs {
        project: Some("demo".into()),
        environment: None,
        json: true,
        yes: false,
        show_secrets: false,
        query: vec![],
        command: ParserHarness::try_parse_from(["intel", "logs", "list", "--follow"])
            .unwrap()
            .command,
    };
    // Observe the second request rather than assuming connection setup and
    // scheduling consume less than 300ms on a cold, busy build machine.
    let observed = async {
        while mock.seen("GET /api/v1/projects/demo/logs").len() < 2 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    tokio::select! {
        result = execute(&client, &args) => panic!("follow ended unexpectedly: {result:?}"),
        result = tokio::time::timeout(Duration::from_secs(30), observed) => result.unwrap(),
    }
}

#[tokio::test]
async fn stream_rejects_non_sse_responses() {
    let mock = Mock::default();
    mock.script("GET /api/v1/activity/stream", 200, json!({"ok":true}));
    let base = mock.start().await;
    let response = reqwest::get(format!("{base}/api/v1/activity/stream"))
        .await
        .unwrap();
    assert!(stream(response, true, false)
        .await
        .unwrap_err()
        .to_string()
        .contains("content type"));
}

#[test]
fn root_cli_registers_intel_and_accepts_global_options_after_verbs() {
    let parsed = crate::Cli::try_parse_from([
        "axiom",
        "intel",
        "logs",
        "list",
        "--follow",
        "--project",
        "Demo project",
        "--environment",
        "dev",
        "--json",
        "--query",
        "limit=20",
    ])
    .unwrap();
    match *parsed.command {
        crate::Commands::Intel(args) => {
            assert!(args.json);
            assert_eq!(args.project.as_deref(), Some("Demo project"));
            assert_eq!(args.environment.as_deref(), Some("dev"));
            assert_eq!(args.query, vec![("limit".into(), "20".into())]);
            assert!(args.command.request().unwrap().follow);
        }
        _ => panic!("expected intel command"),
    }
}

#[test]
fn otlp_create_snippet_is_redacted() {
    // OTLPConnection embeds OTLPSource: these fields are flattened by Go JSON.
    let raw = json!({"id":"s1","name":"worker","endpoint":"https://otlp.example",
        "dsn":"otlp-secret-value", "acore_snippet":"telemetry { dsn: otlp-secret-value }",
        "security_notice":"Credentials are shown only once."});
    let mut value = raw.clone();
    redact(&mut value);
    assert!(!serde_json::to_string(&value)
        .unwrap()
        .contains("otlp-secret-value"));
    assert!(!human_value(&value).contains("otlp-secret-value"));
    assert_eq!(value["name"], "worker");
    // Explicit --show-secrets keeps the original response for both formats.
    assert!(serde_json::to_string(&raw)
        .unwrap()
        .contains("otlp-secret-value"));
    assert!(human_value(&raw).contains("otlp-secret-value"));
}

#[test]
fn human_main_fields_use_plain_words() {
    let text = human_value(
        &json!({"name":"Shop","default_branch":"main","enabled":true,"status":"healthy"}),
    );
    assert!(text.contains("Name: Shop"));
    assert!(text.contains("Default branch: main"));
    assert!(text.contains("Enabled: yes"));
    assert!(text.contains("Status: healthy"));
    assert!(!text.contains('{'));
}

#[test]
fn logs_interval_defaults_to_fifteen_seconds_and_accepts_override() {
    for (argv, seconds) in [
        (vec!["intel", "logs", "list", "--follow"], 15),
        (
            vec!["intel", "logs", "list", "--follow", "--interval", "3"],
            3,
        ),
    ] {
        assert_eq!(
            ParserHarness::try_parse_from(argv)
                .unwrap()
                .command
                .poll_interval(),
            Duration::from_secs(seconds)
        );
    }
    assert!(ParserHarness::try_parse_from(["intel", "logs", "list", "--interval", "0"]).is_err());
}

#[tokio::test]
async fn literal_project_placeholder_identifier_is_not_rewritten() {
    let mock = Mock::default();
    mock.script("GET /api/v1/projects", 200, project_fixture());
    let route = "GET /api/v1/projects/demo/traces/%7Bproject%7D";
    mock.script(route, 200, json!({"id":"{project}"}));
    let base = mock.start().await;
    let client = CloudClient::for_loopback(&base, "test".into(), "refresh".into()).unwrap();
    let args = IntelArgs {
        project: Some("demo".into()),
        environment: None,
        json: true,
        yes: false,
        show_secrets: false,
        query: vec![],
        command: ParserHarness::try_parse_from(["intel", "traces", "get", "{project}"])
            .unwrap()
            .command,
    };
    execute(&client, &args).await.unwrap();
    assert_eq!(mock.seen(route).len(), 1);
}
