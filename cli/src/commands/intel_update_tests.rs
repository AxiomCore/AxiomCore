//! Regression tests use the public command execution path and backend body schemas.
use super::tests::ParserHarness;
use crate::commands::{
    cloud_mock::Mock,
    intel::{execute, IntelArgs},
};
use axiom_cloud::CloudClient;
use clap::Parser;
use serde_json::json;

fn args(argv: &[&str]) -> IntelArgs {
    IntelArgs {
        project: Some("demo".into()),
        environment: None,
        json: true,
        yes: true,
        show_secrets: false,
        query: vec![],
        command: ParserHarness::try_parse_from(argv).unwrap().command,
    }
}
fn fixture(mock: &Mock) {
    mock.script("GET /api/v1/projects", 200, json!([{"id":"project-id","name":"Demo","slug":"demo","visibility":"private","created_at":"2026-11-13","updated_at":"2026-11-13"}]));
}
async fn client(mock: &Mock) -> CloudClient {
    CloudClient::for_loopback(&mock.start().await, "test".into(), "refresh".into()).unwrap()
}

#[tokio::test]
async fn settings_typed_update_preserves_description_branch_and_visibility() {
    let mock = Mock::default();
    fixture(&mock);
    mock.script("GET /api/v1/projects/demo", 200, json!({"name":"Demo","description":"Keep this","default_branch":"production","visibility":"private","id":"ignored"}));
    mock.script(
        "PATCH /api/v1/projects/demo/settings",
        200,
        json!({"ok":true}),
    );
    execute(
        &client(&mock).await,
        &args(&["intel", "settings", "update", "--name", "Shop"]),
    )
    .await
    .unwrap();
    let seen = mock.seen("PATCH /api/v1/projects/demo/settings");
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].body,
        json!({"name":"Shop","description":"Keep this","default_branch":"production","visibility":"private"})
    );
    assert_eq!(mock.seen("GET /api/v1/projects/demo").len(), 1);
}

#[tokio::test]
async fn policy_typed_update_preserves_enabled_and_other_settings() {
    let mock = Mock::default();
    fixture(&mock);
    mock.script("GET /api/v1/projects/demo/alert-policies", 200, json!([
        {"kind":"error_rate","enabled":true,"window_seconds":900,"threshold":1.5,"minimum_samples":100,"severity":"critical","configured":true},
        {"kind":"latency","enabled":false,"window_seconds":60,"threshold":5,"minimum_samples":1,"severity":"warning"}
    ]));
    mock.script(
        "PUT /api/v1/projects/demo/alert-policies/error_rate",
        200,
        json!({"ok":true}),
    );
    execute(
        &client(&mock).await,
        &args(&[
            "intel",
            "alert-policies",
            "update",
            "error_rate",
            "--threshold",
            "2.5",
        ]),
    )
    .await
    .unwrap();
    let seen = mock.seen("PUT /api/v1/projects/demo/alert-policies/error_rate");
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].body,
        json!({"enabled":true,"window_seconds":900,"threshold":2.5,"minimum_samples":100,"severity":"critical"})
    );
    assert_eq!(
        mock.seen("GET /api/v1/projects/demo/alert-policies").len(),
        1
    );
}

#[tokio::test]
async fn settings_can_explicitly_clear_description() {
    let mock = Mock::default();
    fixture(&mock);
    mock.script(
        "GET /api/v1/projects/demo",
        200,
        json!({"name":"Demo","description":"Old","default_branch":"main","visibility":"public"}),
    );
    mock.script(
        "PATCH /api/v1/projects/demo/settings",
        200,
        json!({"ok":true}),
    );
    execute(
        &client(&mock).await,
        &args(&["intel", "settings", "update", "--description", ""]),
    )
    .await
    .unwrap();
    assert_eq!(
        mock.seen("PATCH /api/v1/projects/demo/settings")[0].body,
        json!({"name":"Demo","description":"","default_branch":"main","visibility":"public"})
    );
}

#[tokio::test]
async fn queue_typed_version_sends_actual_backend_shape() {
    for (argv, expected) in [
        (
            vec!["intel", "test-runs", "queue", "--version", "1.2.3"],
            json!({"version":"1.2.3"}),
        ),
        (vec!["intel", "test-runs", "queue"], json!({})),
    ] {
        let mock = Mock::default();
        fixture(&mock);
        mock.script(
            "POST /api/v1/projects/demo/test-runs",
            202,
            json!({"state":"queued"}),
        );
        execute(&client(&mock).await, &args(&argv)).await.unwrap();
        let seen = mock.seen("POST /api/v1/projects/demo/test-runs");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].body, expected);
    }
}

#[test]
fn typed_mutations_reject_raw_body_and_unknown_or_wrong_case_flags() {
    for command in [
        vec!["intel", "settings", "update"],
        vec!["intel", "alert-policies", "update", "error_rate"],
        vec!["intel", "test-runs", "queue"],
    ] {
        for suffix in [
            vec!["--body", r#"{"Name":"Shop"}"#],
            vec!["--Name", "Shop"],
            vec!["--bogus", "x"],
        ] {
            let mut argv = command.clone();
            argv.extend(suffix);
            assert!(ParserHarness::try_parse_from(&argv).is_err(), "{argv:?}");
        }
    }
}

#[tokio::test]
async fn incomplete_current_settings_fail_without_writing() {
    let mock = Mock::default();
    fixture(&mock);
    mock.script("GET /api/v1/projects/demo", 200, json!({"name":"Demo"}));
    assert!(execute(
        &client(&mock).await,
        &args(&["intel", "settings", "update", "--name", "Shop"])
    )
    .await
    .is_err());
    assert!(mock.seen("PATCH /api/v1/projects/demo/settings").is_empty());
}

#[test]
fn policy_typed_values_reject_invalid_numbers_and_keep_explicit_false() {
    for (flag, value) in [
        ("--threshold", "NaN"),
        ("--threshold", "inf"),
        ("--window-seconds", "0"),
        ("--minimum-samples", "0"),
    ] {
        assert!(ParserHarness::try_parse_from([
            "intel",
            "alert-policies",
            "update",
            "error_rate",
            flag,
            value
        ])
        .is_err());
    }
    let parsed = ParserHarness::try_parse_from([
        "intel",
        "alert-policies",
        "update",
        "error_rate",
        "--enabled",
        "false",
    ])
    .unwrap();
    assert_eq!(
        parsed.command.request().unwrap().body,
        Some(json!({"enabled": false}))
    );
}

#[tokio::test]
async fn unconfigured_policy_requires_all_initial_values_before_write() {
    let mock = Mock::default();
    fixture(&mock);
    mock.script("GET /api/v1/projects/demo/alert-policies", 200, json!([
        {"kind":"error_rate","configured":false,"enabled":false,"window_seconds":0,"threshold":0,"minimum_samples":0,"severity":"","updated_at":""}
    ]));
    mock.script(
        "PUT /api/v1/projects/demo/alert-policies/error_rate",
        200,
        json!({"ok":true}),
    );
    let cloud = client(&mock).await;
    let error = execute(
        &cloud,
        &args(&[
            "intel",
            "alert-policies",
            "update",
            "error_rate",
            "--threshold",
            "2.5",
        ]),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("not configured"));
    for missing in [
        "--enabled",
        "--window-seconds",
        "--minimum-samples",
        "--severity",
    ] {
        assert!(error.contains(missing), "{error}");
    }
    assert!(mock
        .seen("PUT /api/v1/projects/demo/alert-policies/error_rate")
        .is_empty());
    execute(
        &cloud,
        &args(&[
            "intel",
            "alert-policies",
            "update",
            "error_rate",
            "--enabled",
            "true",
            "--window-seconds",
            "300",
            "--threshold",
            "2.5",
            "--minimum-samples",
            "10",
            "--severity",
            "warning",
        ]),
    )
    .await
    .unwrap();
    let seen = mock.seen("PUT /api/v1/projects/demo/alert-policies/error_rate");
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].body,
        json!({"enabled":true,"window_seconds":300,"threshold":2.5,"minimum_samples":10,"severity":"warning"})
    );
}
