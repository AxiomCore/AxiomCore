use super::*;
use crate::commands::auth::Action;
use crate::commands::cloud_mock::Mock;
use axiom_cloud::CloudClient;
use std::io::IsTerminal;

const ENV: &str = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
const USER: &str = "3e4f5a6b-7c8d-4e9f-8a0b-1c2d3e4f5a6b";
const FACTOR: &str = "4f5a6b7c-8d9e-4f0a-9b1c-2d3e4f5a6b7c";

fn args(json: bool) -> AuthArgs {
    AuthArgs { project: Some("demo".into()), environment: ENV.into(), json, action: Action::Status }
}

async fn fixture() -> (Mock, Target) {
    let mock = Mock::default();
    let base = mock.start().await;
    let target = Target { client: CloudClient::for_loopback(&base, "access".into(), "refresh".into()).unwrap(), slug: "demo".into(), environment: ENV.into() };
    (mock, target)
}

fn admin_route(method: &str, rest: &str) -> String {
    format!("{method} /api/v1/projects/demo/auth/environments/{ENV}/{rest}")
}

fn exit_code(error: &anyhow::Error) -> Option<i32> {
    error.downcast_ref::<Exit>().map(|e| e.code)
}

#[tokio::test]
async fn delete_sends_a_bodyless_delete_and_needs_confirmation() {
    let (mock, target) = fixture().await;
    let route = admin_route("DELETE", &format!("users/{USER}"));
    mock.script(&route, 200, json!({"deleted": true, "purged": false}));
    users_with(&args(true), &target, &UsersAdminAction::Delete { user_id: USER.into(), yes: true }).await.unwrap();
    let seen = mock.seen(&route);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, Value::Null);
    assert_eq!(seen[0].query, None);
    // A non-UUID id is a usage error before any request.
    let bad = users_with(&args(true), &target, &UsersAdminAction::Delete { user_id: "Not-A-UUID".into(), yes: true }).await.unwrap_err();
    assert_eq!(exit_code(&bad), Some(2));
    // Without --yes and without a terminal, nothing is sent.
    if !std::io::stdin().is_terminal() {
        let refused = users_with(&args(true), &target, &UsersAdminAction::Delete { user_id: USER.into(), yes: false }).await.unwrap_err();
        assert_eq!(exit_code(&refused), Some(2));
        assert_eq!(mock.seen(&route).len(), 1);
    }
}

#[tokio::test]
async fn sessions_reads_the_users_sessions() {
    let (mock, target) = fixture().await;
    let route = admin_route("GET", &format!("users/{USER}/sessions"));
    mock.script(&route, 200, json!({"sessions": [{"id": FACTOR, "status": "active", "auth_method": "password", "ended_at": null}]}));
    users_with(&args(false), &target, &UsersAdminAction::Sessions { user_id: USER.into() }).await.unwrap();
    let seen = mock.seen(&route);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, Value::Null);
    assert_eq!(seen[0].query, None);
}

#[tokio::test]
async fn export_writes_a_new_private_file_and_never_replaces_one() {
    let (mock, target) = fixture().await;
    let route = admin_route("POST", &format!("users/{USER}/export"));
    let document = json!({"format": "axiom-auth-user-export/v1", "user": {"id": USER, "email": "jane@example.com"}, "events": [], "events_truncated": false, "session_token": "SYNTHETIC_TOKEN_CANARY"});
    mock.script(&route, 200, document);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export.json");
    users_with(&args(false), &target, &UsersAdminAction::Export { user_id: USER.into(), output: Some(path.clone()) }).await.unwrap();
    let seen = mock.seen(&route);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, json!({}));
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    // The address is the user's data and stays; secret-like keys never reach the file.
    assert_eq!(written["user"]["email"], "jane@example.com");
    assert_eq!(written["format"], "axiom-auth-user-export/v1");
    assert!(written.get("session_token").is_none());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // An existing file is refused before the request (an export is audited).
    let again = users_with(&args(false), &target, &UsersAdminAction::Export { user_id: USER.into(), output: Some(path.clone()) }).await.unwrap_err();
    assert_eq!(exit_code(&again), Some(2));
    // Neither --output nor --json: refused, nothing sent.
    let nowhere = users_with(&args(false), &target, &UsersAdminAction::Export { user_id: USER.into(), output: None }).await.unwrap_err();
    assert_eq!(exit_code(&nowhere), Some(2));
    assert_eq!(mock.seen(&route).len(), 1);
    // --json prints it.
    users_with(&args(true), &target, &UsersAdminAction::Export { user_id: USER.into(), output: None }).await.unwrap();
    assert_eq!(mock.seen(&route).len(), 2);
}

#[tokio::test]
async fn send_verification_posts_an_empty_body_and_explains_a_conflict() {
    let (mock, target) = fixture().await;
    let route = admin_route("POST", &format!("users/{USER}/verification"));
    mock.script(&route, 202, json!({"sent": true}));
    mock.script(&route, 409, json!({"code": "AXAUTH_CONFLICT"}));
    users_with(&args(true), &target, &UsersAdminAction::SendVerification { user_id: USER.into() }).await.unwrap();
    let seen = mock.seen(&route);
    assert_eq!(seen[0].body, json!({}));
    assert_eq!(seen[0].query, None);
    let conflict = users_with(&args(true), &target, &UsersAdminAction::SendVerification { user_id: USER.into() }).await.unwrap_err();
    assert_eq!(exit_code(&conflict), Some(4));
    assert!(conflict.to_string().contains("already verified"));
}

#[tokio::test]
async fn remove_factor_deletes_one_factor() {
    let (mock, target) = fixture().await;
    let route = admin_route("DELETE", &format!("users/{USER}/factors/{FACTOR}"));
    mock.script(&route, 200, json!({"removed": true}));
    users_with(&args(true), &target, &UsersAdminAction::RemoveFactor { user_id: USER.into(), factor_id: FACTOR.into(), yes: true }).await.unwrap();
    let seen = mock.seen(&route);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, Value::Null);
    let bad = users_with(&args(true), &target, &UsersAdminAction::RemoveFactor { user_id: USER.into(), factor_id: "abc".into(), yes: true }).await.unwrap_err();
    assert_eq!(exit_code(&bad), Some(2));
    if !std::io::stdin().is_terminal() {
        assert!(users_with(&args(true), &target, &UsersAdminAction::RemoveFactor { user_id: USER.into(), factor_id: FACTOR.into(), yes: false }).await.is_err());
    }
    assert_eq!(mock.seen(&route).len(), 1);
}

#[tokio::test]
async fn events_list_sends_the_dashboard_filters() {
    let (mock, target) = fixture().await;
    let route = admin_route("GET", "events");
    mock.script(&route, 200, json!({"events": [{"id": FACTOR, "occurred_at": "2026-11-01T00:00:00Z", "type": "sign_in", "user_id": USER, "actor": "user", "detail": {"method": "password"}}], "next_cursor": "b2NjdXJyZWQ"}));
    let list = EventsAction::List { event_type: Some("sign_in".into()), user_id: Some(USER.into()), limit: Some(50), cursor: Some("b2NjdXJyZWQ".into()) };
    events_with(&args(false), &target, &list).await.unwrap();
    let seen = mock.seen(&route);
    assert_eq!(seen[0].query.as_deref(), Some(format!("limit=50&cursor=b2NjdXJyZWQ&user_id={USER}&type=sign_in").as_str()));
    assert_eq!(seen[0].body, Value::Null);
    events_with(&args(true), &target, &EventsAction::List { event_type: None, user_id: None, limit: None, cursor: None }).await.unwrap();
    assert_eq!(mock.seen(&route)[1].query, None);
    // Bad filters are usage errors before any request.
    let bad_user = events_with(&args(true), &target, &EventsAction::List { event_type: None, user_id: Some("jane".into()), limit: None, cursor: None }).await.unwrap_err();
    assert_eq!(exit_code(&bad_user), Some(2));
    let bad_cursor = events_with(&args(true), &target, &EventsAction::List { event_type: None, user_id: None, limit: None, cursor: Some("a b".into()) }).await.unwrap_err();
    assert_eq!(exit_code(&bad_cursor), Some(2));
    assert_eq!(mock.seen(&route).len(), 2);
}

#[test]
fn verbs_parse_under_users_and_events() {
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        auth: AuthArgs,
    }
    let cli = Cli::try_parse_from(["axiom", "users", "remove-factor", USER, FACTOR, "--yes"]).unwrap();
    assert!(matches!(cli.auth.action, Action::Users { action: crate::commands::auth::UsersAction::Admin(UsersAdminAction::RemoveFactor { yes: true, .. }) }));
    let cli = Cli::try_parse_from(["axiom", "users", "send-verification", USER]).unwrap();
    assert!(matches!(cli.auth.action, Action::Users { action: crate::commands::auth::UsersAction::Admin(UsersAdminAction::SendVerification { .. }) }));
    let cli = Cli::try_parse_from(["axiom", "events", "list", "--type", "user_deleted", "--limit", "10"]).unwrap();
    assert!(matches!(cli.auth.action, Action::Events { action: EventsAction::List { limit: Some(10), .. } }));
    assert!(Cli::try_parse_from(["axiom", "events", "list", "--type", "nope"]).is_err());
    assert!(Cli::try_parse_from(["axiom", "events", "list", "--limit", "101"]).is_err());
}
