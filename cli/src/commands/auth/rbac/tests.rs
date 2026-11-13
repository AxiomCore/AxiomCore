use super::*;
use crate::commands::auth::Action;
use crate::commands::cloud_mock::Mock;
use axiom_cloud::CloudClient;
use std::io::IsTerminal;

const ENV: &str = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
const ROLE: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
const READ: &str = "2d3e4f5a-6b7c-4d8e-9f0a-1b2c3d4e5f60";
const WRITE: &str = "3e4f5a6b-7c8d-4e9f-8a1b-2c3d4e5f6a71";
const USER: &str = "4f5a6b7c-8d9e-4f0a-9b2c-3d4e5f6a7b82";
const ORG: &str = "5a6b7c8d-9e0f-4a1b-8c3d-4e5f6a7b8c93";
const GRANT: &str = "6b7c8d9e-0f1a-4b2c-9d4e-5f6a7b8c9da4";

fn args() -> AuthArgs {
    AuthArgs { project: Some("demo".into()), environment: ENV.into(), json: true, action: Action::Status }
}

async fn fixture() -> (Mock, Target) {
    let mock = Mock::default();
    let base = mock.start().await;
    let target = Target { client: CloudClient::for_loopback(&base, "access".into(), "refresh".into()).unwrap(), slug: "demo".into(), environment: ENV.into() };
    (mock, target)
}

fn route(method: &str, rest: &str) -> String {
    format!("{method} /api/v1/projects/demo/auth/environments/{ENV}/{rest}")
}

fn editor(revision: i64, permission_ids: &[&str]) -> Value {
    json!({"id": ROLE, "name": "editor", "description": "Edits posts", "revision": revision, "permission_ids": permission_ids})
}

fn script_lists(mock: &Mock) {
    mock.script(&route("GET", "roles"), 200, json!({"roles": [editor(3, &[READ])]}));
    mock.script(&route("GET", "permissions"), 200, json!({"permissions": [{"id": READ, "name": "posts.read", "description": ""}, {"id": WRITE, "name": "posts.write", "description": ""}]}));
}

#[tokio::test]
async fn roles_list_and_create_with_permissions_by_name() {
    let (mock, target) = fixture().await;
    script_lists(&mock);
    roles_with(&args(), &target, &RolesAction::List).await.unwrap();
    let seen = mock.seen(&route("GET", "roles"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].query, None);

    mock.script(&route("POST", "roles"), 201, json!({"id": ROLE, "name": "editor", "description": "Edits posts", "revision": 1, "permission_ids": []}));
    mock.script(&route("PUT", &format!("roles/{ROLE}/permissions")), 200, editor(2, &[READ, WRITE]));
    let create = RolesAction::Create { name: "editor".into(), description: " Edits posts ".into(), permissions: vec!["posts.read".into(), WRITE.into(), "posts.read".into()] };
    roles_with(&args(), &target, &create).await.unwrap();
    assert_eq!(mock.seen(&route("POST", "roles"))[0].body, json!({"name": "editor", "description": "Edits posts"}));
    // Names resolve through GET permissions; repeats are dropped; the revision comes from the created role.
    assert_eq!(mock.seen(&route("GET", "permissions")).len(), 1);
    assert_eq!(mock.seen(&route("PUT", &format!("roles/{ROLE}/permissions")))[0].body, json!({"permission_ids": [READ, WRITE], "revision": 1}));

    // Without --permission only the POST is sent.
    let plain = RolesAction::Create { name: "viewer".into(), description: String::new(), permissions: vec![] };
    roles_with(&args(), &target, &plain).await.unwrap();
    assert_eq!(mock.seen(&route("POST", "roles"))[1].body, json!({"name": "viewer", "description": ""}));
    assert_eq!(mock.seen(&route("PUT", &format!("roles/{ROLE}/permissions"))).len(), 1);
}

#[tokio::test]
async fn an_unknown_permission_name_creates_nothing() {
    let (mock, target) = fixture().await;
    script_lists(&mock);
    let create = RolesAction::Create { name: "editor".into(), description: String::new(), permissions: vec!["posts.delete".into()] };
    let error = roles_with(&args(), &target, &create).await.unwrap_err();
    assert_eq!(error.downcast_ref::<crate::commands::auth::Exit>().unwrap().code, 2);
    assert!(mock.seen(&route("POST", "roles")).is_empty());
}

#[tokio::test]
async fn roles_update_and_set_permissions_send_the_current_revision() {
    let (mock, target) = fixture().await;
    script_lists(&mock);
    mock.script(&route("PATCH", &format!("roles/{ROLE}")), 200, json!({"id": ROLE, "name": "editor", "description": "", "revision": 4, "permission_ids": [READ]}));
    roles_with(&args(), &target, &RolesAction::Update { role: "editor".into(), description: "".into() }).await.unwrap();
    assert_eq!(mock.seen(&route("PATCH", &format!("roles/{ROLE}")))[0].body, json!({"description": "", "revision": 3}));

    mock.script(&route("PUT", &format!("roles/{ROLE}/permissions")), 200, editor(4, &[WRITE]));
    let set = RolesAction::SetPermissions { role: ROLE.to_uppercase(), permissions: vec!["posts.write".into()], none: false };
    roles_with(&args(), &target, &set).await.unwrap();
    assert_eq!(mock.seen(&route("PUT", &format!("roles/{ROLE}/permissions")))[0].body, json!({"permission_ids": [WRITE], "revision": 3}));

    let clear = RolesAction::SetPermissions { role: "editor".into(), permissions: vec![], none: true };
    roles_with(&args(), &target, &clear).await.unwrap();
    assert_eq!(mock.seen(&route("PUT", &format!("roles/{ROLE}/permissions")))[1].body, json!({"permission_ids": [], "revision": 3}));

    // An unknown role is a usage error before any write.
    let missing = RolesAction::Update { role: "admin".into(), description: "x".into() };
    assert!(roles_with(&args(), &target, &missing).await.is_err());
    assert_eq!(mock.seen(&route("PATCH", &format!("roles/{ROLE}"))).len(), 1);
}

#[tokio::test]
async fn deletes_resolve_names_and_need_confirmation() {
    let (mock, target) = fixture().await;
    script_lists(&mock);
    mock.script(&route("DELETE", &format!("roles/{ROLE}")), 200, json!({"deleted": true}));
    mock.script(&route("DELETE", &format!("permissions/{WRITE}")), 200, json!({"deleted": true}));
    roles_with(&args(), &target, &RolesAction::Delete { role: "editor".into(), yes: true }).await.unwrap();
    let deleted = mock.seen(&route("DELETE", &format!("roles/{ROLE}")));
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].body, Value::Null);
    // A UUID needs no lookup.
    roles_with(&args(), &target, &RolesAction::Delete { role: ROLE.into(), yes: true }).await.unwrap();
    assert_eq!(mock.seen(&route("GET", "roles")).len(), 1);

    permissions_with(&args(), &target, &PermissionsAction::Delete { permission: "posts.write".into(), yes: true }).await.unwrap();
    assert_eq!(mock.seen(&route("DELETE", &format!("permissions/{WRITE}"))).len(), 1);

    if !std::io::stdin().is_terminal() {
        assert!(roles_with(&args(), &target, &RolesAction::Delete { role: "editor".into(), yes: false }).await.is_err());
        assert!(permissions_with(&args(), &target, &PermissionsAction::Delete { permission: WRITE.into(), yes: false }).await.is_err());
        assert!(assignments_with(&args(), &target, &AssignmentsAction::Remove { id: GRANT.into(), yes: false }).await.is_err());
        assert_eq!(mock.seen(&route("DELETE", &format!("roles/{ROLE}"))).len(), 2);
        assert_eq!(mock.seen(&route("DELETE", &format!("permissions/{WRITE}"))).len(), 1);
        assert!(mock.seen(&route("DELETE", &format!("assignments/{GRANT}"))).is_empty());
    }
}

#[tokio::test]
async fn permissions_list_and_create() {
    let (mock, target) = fixture().await;
    script_lists(&mock);
    permissions_with(&args(), &target, &PermissionsAction::List).await.unwrap();
    assert_eq!(mock.seen(&route("GET", "permissions")).len(), 1);
    mock.script(&route("POST", "permissions"), 201, json!({"id": WRITE, "name": "posts.write", "description": "Write posts"}));
    permissions_with(&args(), &target, &PermissionsAction::Create { name: "posts.write".into(), description: "Write posts".into() }).await.unwrap();
    assert_eq!(mock.seen(&route("POST", "permissions"))[0].body, json!({"name": "posts.write", "description": "Write posts"}));
}

#[tokio::test]
async fn assignments_list_add_and_remove() {
    let (mock, target) = fixture().await;
    script_lists(&mock);
    let grant = json!({"id": GRANT, "user_id": USER, "role_id": ROLE, "organization_id": ORG, "valid_from": "2026-11-13T00:00:00Z", "valid_until": null, "granted_by": "admin:x"});
    mock.script(&route("GET", "assignments"), 200, json!({"assignments": [grant.clone()]}));
    assignments_with(&args(), &target, &AssignmentsAction::List { user: USER.to_uppercase() }).await.unwrap();
    assert_eq!(mock.seen(&route("GET", "assignments"))[0].query.as_deref(), Some(format!("user_id={USER}").as_str()));

    mock.script(&route("POST", "assignments"), 201, grant);
    let add = AssignmentsAction::Add { user: USER.into(), role: "editor".into(), organization: Some(ORG.into()), valid_until: Some("2099-01-31T01:00:00+01:00".into()) };
    assignments_with(&args(), &target, &add).await.unwrap();
    assert_eq!(mock.seen(&route("POST", "assignments"))[0].body, json!({"user_id": USER, "role_id": ROLE, "organization_id": ORG, "valid_until": "2099-01-31T00:00:00.000Z"}));
    // Environment-wide, role by UUID: no lookup, no optional fields.
    let env_wide = AssignmentsAction::Add { user: USER.into(), role: ROLE.into(), organization: None, valid_until: None };
    assignments_with(&args(), &target, &env_wide).await.unwrap();
    assert_eq!(mock.seen(&route("POST", "assignments"))[1].body, json!({"user_id": USER, "role_id": ROLE}));
    assert_eq!(mock.seen(&route("GET", "roles")).len(), 1);

    mock.script(&route("DELETE", &format!("assignments/{GRANT}")), 200, json!({"deleted": true}));
    assignments_with(&args(), &target, &AssignmentsAction::Remove { id: GRANT.into(), yes: true }).await.unwrap();
    assert_eq!(mock.seen(&route("DELETE", &format!("assignments/{GRANT}"))).len(), 1);
}

#[tokio::test]
async fn malformed_ids_are_usage_errors_without_a_request() {
    let (mock, target) = fixture().await;
    for action in [
        AssignmentsAction::List { user: "alice".into() },
        AssignmentsAction::Add { user: USER.into(), role: ROLE.into(), organization: Some("acme".into()), valid_until: None },
        AssignmentsAction::Add { user: USER.into(), role: ROLE.into(), organization: None, valid_until: Some("2020-01-01T00:00:00Z".into()) },
        AssignmentsAction::Add { user: USER.into(), role: ROLE.into(), organization: None, valid_until: Some("tomorrow".into()) },
        AssignmentsAction::Remove { id: "../users".into(), yes: true },
    ] {
        let error = assignments_with(&args(), &target, &action).await.unwrap_err();
        assert_eq!(error.downcast_ref::<crate::commands::auth::Exit>().unwrap().code, 2, "{action:?}");
    }
    assert!(mock.seen(&route("GET", "assignments")).is_empty());
    assert!(mock.seen(&route("POST", "assignments")).is_empty());
}

#[test]
fn names_follow_the_backend_patterns() {
    assert!(valid_role_name("editor"));
    assert!(valid_role_name("support_l2-eu"));
    assert!(!valid_role_name("Editor"));
    assert!(!valid_role_name("2fa"));
    assert!(!valid_role_name("posts.read"));
    assert!(!valid_role_name(&"a".repeat(65)));
    assert!(valid_permission_name("posts.read"));
    assert!(valid_permission_name("billing:invoices.write"));
    assert!(!valid_permission_name("posts.*"));
    assert!(!valid_permission_name(""));
    assert!(description(&"x".repeat(257)).is_err());
    assert_eq!(description(" hi ").unwrap(), "hi");
}

#[test]
fn set_permissions_needs_a_permission_or_none() {
    use clap::Parser;
    #[derive(Parser, Debug)]
    struct Cli {
        #[command(subcommand)]
        action: RolesAction,
    }
    assert!(Cli::try_parse_from(["roles", "set-permissions", "editor"]).is_err());
    assert!(Cli::try_parse_from(["roles", "set-permissions", "editor", "--none", "--permission", "posts.read"]).is_err());
    assert!(Cli::try_parse_from(["roles", "set-permissions", "editor", "--none"]).is_ok());
    assert!(Cli::try_parse_from(["roles", "set-permissions", "editor", "--permission", "a", "--permission", "b"]).is_ok());
}
