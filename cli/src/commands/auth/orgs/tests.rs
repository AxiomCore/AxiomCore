use super::*;
use crate::commands::auth::Action;
use crate::commands::cloud_mock::Mock;
use axiom_cloud::CloudClient;
use std::io::IsTerminal;

const ENV: &str = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
const ORG: &str = "3a4b5c6d-7e8f-4a9b-8c0d-1e2f3a4b5c6d";
const USER: &str = "4b5c6d7e-8f9a-4b0c-9d1e-2f3a4b5c6d7e";
const INVITE: &str = "5c6d7e8f-9a0b-4c1d-8e2f-3a4b5c6d7e8f";

fn args() -> AuthArgs {
    AuthArgs { project: Some("demo".into()), environment: ENV.into(), json: true, action: Action::Status }
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

fn org(id: &str, slug: &str, revision: u64) -> Value {
    json!({"id": id, "slug": slug, "name": "Acme", "status": "active", "revision": revision, "created_at": "2026-11-01T00:00:00Z"})
}

#[tokio::test]
async fn organizations_list_and_create() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("GET", "organizations"), 200, json!({"organizations": [org(ORG, "acme", 4)], "next_cursor": "c2"}));
    organizations_with(&args(), &target, &OrganizationsAction::List { limit: Some(20), cursor: Some("c1".into()) }).await.unwrap();
    let seen = mock.seen(&admin_route("GET", "organizations"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].query.as_deref(), Some("limit=20&cursor=c1"));

    mock.script(&admin_route("POST", "organizations"), 201, org(ORG, "acme", 5));
    organizations_with(&args(), &target, &OrganizationsAction::Create { slug: " acme ".into(), name: " Acme Inc ".into() }).await.unwrap();
    let seen = mock.seen(&admin_route("POST", "organizations"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, json!({"slug": "acme", "name": "Acme Inc"}));
    assert_eq!(seen[0].query, None);

    // Bad slugs and names are refused before any request.
    let long = "x".repeat(129);
    for (slug, name) in [("Acme", "Acme"), ("-acme", "Acme"), ("acme_1", "Acme"), ("acme", " "), ("acme", long.as_str())] {
        let action = OrganizationsAction::Create { slug: slug.into(), name: name.to_string() };
        assert!(organizations_with(&args(), &target, &action).await.is_err(), "{slug} {name}");
    }
    assert!(organizations_with(&args(), &target, &OrganizationsAction::List { limit: None, cursor: Some("bad cursor".into()) }).await.is_err());
    assert_eq!(mock.seen(&admin_route("POST", "organizations")).len(), 1);
    assert_eq!(mock.seen(&admin_route("GET", "organizations")).len(), 1);
}

#[tokio::test]
async fn organization_update_reads_the_revision_and_resolves_slugs_across_pages() {
    let (mock, target) = fixture().await;
    let other = "6d7e8f9a-0b1c-4d2e-9f3a-4b5c6d7e8f9a";
    mock.script(&admin_route("GET", "organizations"), 200, json!({"organizations": [org(other, "alpha", 9)], "next_cursor": "p2"}));
    mock.script(&admin_route("GET", "organizations"), 200, json!({"organizations": [org(ORG, "beta", 9)], "next_cursor": null}));
    mock.script(&admin_route("PATCH", &format!("organizations/{ORG}")), 200, json!({"id": ORG, "slug": "beta", "name": "Acme", "status": "active", "revision": 10}));
    organizations_with(&args(), &target, &OrganizationsAction::Update { organization: "beta".into(), status: "active".into(), yes: false }).await.unwrap();
    let lists = mock.seen(&admin_route("GET", "organizations"));
    assert_eq!(lists.iter().map(|s| s.query.clone().unwrap_or_default()).collect::<Vec<_>>(), vec!["limit=100", "limit=100&cursor=p2"]);
    let patch = mock.seen(&admin_route("PATCH", &format!("organizations/{ORG}")));
    assert_eq!(patch.len(), 1);
    assert_eq!(patch[0].body, json!({"status": "active", "revision": 9}));

    // Suspending by UUID with --yes; the list repeats its last page (no next cursor).
    organizations_with(&args(), &target, &OrganizationsAction::Update { organization: ORG.into(), status: "suspended".into(), yes: true }).await.unwrap();
    let patch = mock.seen(&admin_route("PATCH", &format!("organizations/{ORG}")));
    assert_eq!(patch[1].body, json!({"status": "suspended", "revision": 9}));

    // An unknown organization sends no PATCH.
    let unknown = OrganizationsAction::Update { organization: "gamma".into(), status: "active".into(), yes: false };
    assert!(organizations_with(&args(), &target, &unknown).await.is_err());
    assert_eq!(mock.seen(&admin_route("PATCH", &format!("organizations/{ORG}"))).len(), 2);
    // Suspending without --yes and without a terminal is refused before any request.
    if !std::io::stdin().is_terminal() {
        let before = mock.seen(&admin_route("GET", "organizations")).len();
        let refused = OrganizationsAction::Update { organization: ORG.into(), status: "suspended".into(), yes: false };
        assert!(organizations_with(&args(), &target, &refused).await.is_err());
        assert_eq!(mock.seen(&admin_route("GET", "organizations")).len(), before);
    }
    // Neither a UUID nor a slug.
    let bad = OrganizationsAction::Update { organization: "Not A Slug".into(), status: "active".into(), yes: true };
    assert!(organizations_with(&args(), &target, &bad).await.is_err());
}

#[tokio::test]
async fn members_list_add_and_remove() {
    let (mock, target) = fixture().await;
    let members = format!("organizations/{ORG}/members");
    let member = format!("organizations/{ORG}/members/{USER}");
    mock.script(&admin_route("GET", &members), 200, json!({"members": [{"organization_id": ORG, "user_id": USER, "status": "active"}], "next_cursor": null}));
    organizations_with(&args(), &target, &OrganizationsAction::Members { organization: ORG.into(), limit: Some(100), cursor: None }).await.unwrap();
    let seen = mock.seen(&admin_route("GET", &members));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].query.as_deref(), Some("limit=100"));
    // A UUID needs no lookup.
    assert!(mock.seen(&admin_route("GET", "organizations")).is_empty());

    mock.script(&admin_route("PUT", &member), 200, json!({"organization_id": ORG, "user_id": USER, "status": "suspended"}));
    organizations_with(&args(), &target, &OrganizationsAction::AddMember { organization: ORG.into(), user: USER.into(), status: "suspended".into() }).await.unwrap();
    let put = mock.seen(&admin_route("PUT", &member));
    assert_eq!(put.len(), 1);
    assert_eq!(put[0].body, json!({"status": "suspended"}));

    // By slug: the organization list resolves the id first.
    mock.script(&admin_route("GET", "organizations"), 200, json!({"organizations": [org(ORG, "acme", 2)], "next_cursor": null}));
    organizations_with(&args(), &target, &OrganizationsAction::AddMember { organization: "acme".into(), user: USER.into(), status: "active".into() }).await.unwrap();
    assert_eq!(mock.seen(&admin_route("PUT", &member))[1].body, json!({"status": "active"}));
    assert_eq!(mock.seen(&admin_route("GET", "organizations")).len(), 1);

    mock.script(&admin_route("DELETE", &member), 200, json!({"deleted": true}));
    organizations_with(&args(), &target, &OrganizationsAction::RemoveMember { organization: ORG.into(), user: USER.into(), yes: true }).await.unwrap();
    let deleted = mock.seen(&admin_route("DELETE", &member));
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].body, Value::Null);

    // A non-UUID user is a usage error and sends nothing.
    let bad = OrganizationsAction::AddMember { organization: ORG.into(), user: "jane@example.com".into(), status: "active".into() };
    let error = organizations_with(&args(), &target, &bad).await.unwrap_err();
    assert_eq!(error.downcast_ref::<Exit>().map(|e| e.code), Some(2));
    if !std::io::stdin().is_terminal() {
        let refused = OrganizationsAction::RemoveMember { organization: ORG.into(), user: USER.into(), yes: false };
        assert!(organizations_with(&args(), &target, &refused).await.is_err());
    }
    assert_eq!(mock.seen(&admin_route("PUT", &member)).len(), 2);
    assert_eq!(mock.seen(&admin_route("DELETE", &member)).len(), 1);
}

#[tokio::test]
async fn invitations_list_resend_and_revoke() {
    let (mock, target) = fixture().await;
    let view = json!({"id": INVITE, "email": "jane@example.com", "status": "pending", "organization_id": ORG, "role_ids": [], "created_at": "2026-11-01T00:00:00Z",
        "expires_at": "2026-11-08T00:00:00Z", "accepted_at": null, "delivery_state": "sent"});
    mock.script(&admin_route("GET", "invitations"), 200, json!({"invitations": [view.clone()], "next_cursor": null}));
    let list = InvitationsAction::List { status: Some("pending".into()), limit: Some(50), cursor: Some("abc_1".into()), reveal_email: false };
    invitations_with(&args(), &target, &list).await.unwrap();
    let seen = mock.seen(&admin_route("GET", "invitations"));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].query.as_deref(), Some("status=pending&limit=50&cursor=abc_1"));
    invitations_with(&args(), &target, &InvitationsAction::List { status: None, limit: None, cursor: None, reveal_email: true }).await.unwrap();
    assert_eq!(mock.seen(&admin_route("GET", "invitations"))[1].query, None);

    let resend = format!("invitations/{INVITE}/resend");
    mock.script(&admin_route("POST", &resend), 200, view.clone());
    invitations_with(&args(), &target, &InvitationsAction::Resend { id: INVITE.into(), reveal_email: false }).await.unwrap();
    let seen = mock.seen(&admin_route("POST", &resend));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, json!({}));

    let revoke = format!("invitations/{INVITE}/revoke");
    let mut revoked = view.clone();
    revoked["status"] = json!("revoked");
    mock.script(&admin_route("POST", &revoke), 200, revoked);
    invitations_with(&args(), &target, &InvitationsAction::Revoke { id: INVITE.into(), yes: true, reveal_email: false }).await.unwrap();
    let seen = mock.seen(&admin_route("POST", &revoke));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, json!({}));

    // Refusals send nothing: a non-UUID id, revoking without --yes off a terminal.
    assert!(invitations_with(&args(), &target, &InvitationsAction::Resend { id: "INV-1".into(), reveal_email: false }).await.is_err());
    if !std::io::stdin().is_terminal() {
        assert!(invitations_with(&args(), &target, &InvitationsAction::Revoke { id: INVITE.into(), yes: false, reveal_email: false }).await.is_err());
    }
    assert_eq!(mock.seen(&admin_route("POST", &resend)).len(), 1);
    assert_eq!(mock.seen(&admin_route("POST", &revoke)).len(), 1);
}

#[test]
fn invitation_addresses_are_masked_unless_revealed() {
    let page = json!({"invitations": [{"id": INVITE, "email": "jane@example.com", "status": "pending"}], "next_cursor": null});
    let mut masked = page.clone();
    sanitize(&mut masked, false);
    assert_eq!(masked["invitations"][0]["email"], "j***@e***.com");
    assert!(!invitation_line(&masked["invitations"][0]).contains("jane@example.com"));
    let mut shown = page;
    sanitize(&mut shown, true);
    assert_eq!(shown["invitations"][0]["email"], "jane@example.com");
}

#[test]
fn slugs_references_and_flags() {
    assert!(valid_slug("a"));
    assert!(valid_slug("acme-2"));
    assert!(valid_slug(&"a".repeat(63)));
    assert!(!valid_slug(&"a".repeat(64)));
    assert!(!valid_slug(""));
    assert!(!valid_slug("-a"));
    assert!(!valid_slug("Acme"));
    assert_eq!(org_ref(ORG).unwrap(), OrgRef::Id(ORG));
    assert_eq!(org_ref("acme").unwrap(), OrgRef::Slug("acme"));
    assert!(org_ref("acme corp").is_err());
    assert!(page_query(Some(10), Some(&"c".repeat(513))).is_err());

    #[derive(clap::Parser)]
    struct Harness {
        #[command(flatten)]
        auth: AuthArgs,
    }
    use clap::Parser;
    assert!(Harness::try_parse_from(["t", "organizations", "list", "--limit", "0"]).is_err());
    assert!(Harness::try_parse_from(["t", "organizations", "list", "--limit", "100"]).is_ok());
    assert!(Harness::try_parse_from(["t", "organizations", "update", "acme", "--status", "deleted"]).is_err());
    assert!(Harness::try_parse_from(["t", "organizations", "update", "acme"]).is_err());
    assert!(Harness::try_parse_from(["t", "organizations", "add-member", "acme", USER]).is_ok());
    assert!(Harness::try_parse_from(["t", "organizations", "update-member", "acme", USER, "--status", "suspended"]).is_ok());
    assert!(Harness::try_parse_from(["t", "organizations", "remove-member", "acme", USER, "--yes"]).is_ok());
    assert!(Harness::try_parse_from(["t", "invitations", "list", "--status", "expired"]).is_err());
    assert!(Harness::try_parse_from(["t", "invitations", "list", "--status", "accepted", "--reveal-email"]).is_ok());
    assert!(Harness::try_parse_from(["t", "invitations", "revoke", INVITE, "--yes"]).is_ok());
    assert!(Harness::try_parse_from(["t", "invitations", "resend", INVITE]).is_ok());
}
