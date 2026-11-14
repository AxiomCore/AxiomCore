use super::*;
use crate::commands::auth::Action;
use crate::commands::cloud_mock::Mock;
use axiom_cloud::CloudClient;

const ENV: &str = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
const CLIENT: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
const CANARY: &str = "SYNTHETIC_CLIENT_SECRET_CANARY";

fn args() -> AuthArgs {
    AuthArgs { project: Some("demo".into()), org: None, environment: ENV.into(), json: true, action: Action::Status }
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

#[tokio::test]
async fn oauth_create_sends_an_idempotency_key_and_writes_the_secret_once() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("POST", "oauth-clients"), 201, json!({"client": {"id": CLIENT, "client_id": "axc_1", "name": "CLI", "type": "confidential", "enabled": false}, "client_secret": CANARY}));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret.txt");
    let sink = secret_sink(Some(&path), true).unwrap();
    let action = OauthAppsAction::Create {
        name: "CLI".into(),
        client_type: "confidential".into(),
        redirect_uris: vec!["https://app.example/cb".into()],
        post_logout_redirect_uris: vec![],
        web_origins: vec![],
        scopes: vec![],
        audiences: vec![],
        permissions: vec![],
        roles: vec![],
        first_party: false,
        can_introspect: true,
        auth_method: None,
        output_secret: Some(path.clone()),
        idempotency_key: Some("0b7d2f8a-61a3-4f4e-9d52-27f2b5f0c111".into()),
    };
    oauth_apps_with(&args(), &target, &action, Some(&sink)).await.unwrap();
    let seen = mock.seen(&admin_route("POST", "oauth-clients"));
    assert_eq!(seen[0].key.as_deref(), Some("0b7d2f8a-61a3-4f4e-9d52-27f2b5f0c111"));
    assert_eq!(seen[0].body["token_endpoint_auth_method"], "client_secret_basic");
    assert_eq!(seen[0].body["allowed_scopes"], json!(["openid"]));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), format!("{CANARY}\n"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // An existing file is never replaced, and --json alone cannot receive a secret.
    assert!(secret_sink(Some(&path), true).is_err());
    assert!(secret_sink(None, true).is_err());
}

#[tokio::test]
async fn rotate_needs_a_secret_in_the_reply_and_updates_send_the_revision() {
    let (mock, target) = fixture().await;
    mock.script(&admin_route("GET", &format!("oauth-clients/{CLIENT}")), 200, json!({"client": {"id": CLIENT, "revision": 7}}));
    mock.script(&admin_route("PATCH", &format!("oauth-clients/{CLIENT}")), 200, json!({"client": {"id": CLIENT, "revision": 8, "enabled": false}}));
    oauth_apps_with(&args(), &target, &OauthAppsAction::Disable { id: CLIENT.into() }, None).await.unwrap();
    assert_eq!(mock.seen(&admin_route("PATCH", &format!("oauth-clients/{CLIENT}")))[0].body, json!({"enabled": false, "revision": 7}));
    mock.script(&admin_route("POST", &format!("oauth-clients/{CLIENT}/secret")), 200, json!({"previous_expires_at": "2026-11-02T00:00:00Z"}));
    let dir = tempfile::tempdir().unwrap();
    let sink = secret_sink(Some(&dir.path().join("s")), true).unwrap();
    let rotate = OauthAppsAction::RotateSecret { id: CLIENT.into(), previous_valid: "90m".into(), output_secret: None };
    assert!(oauth_apps_with(&args(), &target, &rotate, Some(&sink)).await.is_err());
    assert_eq!(mock.seen(&admin_route("POST", &format!("oauth-clients/{CLIENT}/secret")))[0].body, json!({"previous_valid_seconds": 5400}));
    let delete = OauthAppsAction::Delete { id: CLIENT.into(), confirm: Some("delete client other".into()) };
    assert!(oauth_apps_with(&args(), &target, &delete, None).await.is_err());
}

#[test]
fn oauth_bodies_and_durations() {
    let public = oauth_create_body("SPA", "public", &["http://127.0.0.1/cb".into()], &[], &["http://127.0.0.1".into()], &["openid".into(), "email".into()], &[], &[], &["editor".into()], false, false, None).unwrap();
    assert_eq!(public["token_endpoint_auth_method"], "none");
    assert_eq!(public["role_ceiling"], json!(["editor"]));
    assert!(public.get("permission_ceiling").is_none());
    assert!(oauth_create_body("SPA", "public", &[], &[], &[], &[], &[], &[], &[], false, false, Some("client_secret_post")).is_err());
    assert_eq!(duration_seconds("24h").unwrap(), 86400);
    assert_eq!(duration_seconds("0").unwrap(), 0);
    assert_eq!(duration_seconds("7d").unwrap(), 604800);
    assert!(duration_seconds("8d").is_err());
    assert!(duration_seconds("1w").is_err());
}

#[test]
fn provider_extras_check_kind_specific_flags() {
    let ms = provider_extras("microsoft", Some("organizations"), &["6e2a1d3f-9b8c-4d7e-8f1a-2b3c4d5e6f70".into()], None, None, false, true).unwrap();
    assert_eq!(ms, json!({"microsoft_tenant": "organizations", "allowed_tenants": ["6e2a1d3f-9b8c-4d7e-8f1a-2b3c4d5e6f70"]}));
    assert!(provider_extras("microsoft", None, &[], None, None, false, true).is_err());
    assert!(provider_extras("microsoft", Some("contoso"), &[], None, None, false, true).is_err());
    assert!(provider_extras("microsoft", Some("common"), &[], Some("ABCDE12345"), None, false, true).is_err());
    let apple = provider_extras("apple", None, &[], Some("ABCDE12345"), Some("KEY1234567"), true, true).unwrap();
    assert_eq!(apple, json!({"apple_team_id": "ABCDE12345", "apple_key_id": "KEY1234567"}));
    assert!(provider_extras("apple", None, &[], Some("ABCDE12345"), Some("KEY1234567"), false, true).is_err());
    assert!(provider_extras("apple", None, &[], Some("abc"), None, false, false).is_err());
    assert!(provider_extras("google", Some("common"), &[], None, None, false, true).is_err());
    assert_eq!(provider_extras("google", None, &[], None, None, false, true).unwrap(), json!({}));
    let dir = tempfile::tempdir().unwrap();
    let p8 = dir.path().join("AuthKey.p8");
    std::fs::write(&p8, "-----BEGIN PRIVATE KEY-----\nSYNTHETIC\n-----END PRIVATE KEY-----\n").unwrap();
    assert!(apple_private_key(&p8).unwrap().contains("SYNTHETIC"));
    std::fs::write(&p8, "not a key").unwrap();
    assert!(apple_private_key(&p8).is_err());
}

#[test]
fn sms_bodies_never_carry_the_secret_and_validate_identifiers() {
    let sid = format!("AC{}", "a".repeat(32));
    let mg = format!("MG{}", "b".repeat(32));
    let body = sms_body(&sid, None, Some(&mg), None, &["US".into()], 100, Some(10), None).unwrap();
    assert_eq!(body["credential_kind"], "auth_token");
    assert!(body.get("secret").is_none());
    let key = format!("SK{}", "c".repeat(32));
    assert_eq!(sms_body(&sid, Some(&key), None, Some("+15551234567"), &["US".into()], 1, None, None).unwrap()["credential_kind"], "api_key");
    assert!(sms_body("AC1", None, Some(&mg), None, &["US".into()], 1, None, None).is_err());
    assert!(sms_body(&sid, None, None, None, &["US".into()], 1, None, None).is_err());
    assert!(sms_body(&sid, None, Some(&mg), None, &["usa".into()], 1, None, None).is_err());
}

#[tokio::test]
async fn webhook_updates_read_the_revision_and_secrets_stay_out_of_output() {
    let (mock, target) = fixture().await;
    let id = "2d3e4f5a-6b7c-4d8e-9f0a-1b2c3d4e5f60";
    mock.script(&admin_route("GET", "webhooks"), 200, json!({"webhooks": [{"id": id, "revision": 3, "url": "https://hooks.example/x"}]}));
    mock.script(&admin_route("PATCH", &format!("webhooks/{id}")), 200, json!({"webhook": {"id": id, "revision": 4}}));
    let update = WebhooksAction::Update { id: id.into(), url: None, events: vec!["user.created".into()], enable: true, disable: false };
    webhooks_with(&args(), &target, &update, None).await.unwrap();
    assert_eq!(mock.seen(&admin_route("PATCH", &format!("webhooks/{id}")))[0].body, json!({"revision": 3, "event_types": ["user.created"], "enabled": true}));
    let mut view = json!({"webhook": {"id": id}, "secret": CANARY});
    let secret = take_secret(&mut view, &["secret"]).unwrap();
    assert_eq!(secret.as_str(), CANARY);
    sanitize(&mut view, true);
    assert!(!view.to_string().contains(CANARY));
    assert!(https_url("http://hooks.example").is_err());
    assert!(https_url("https://user:pw@hooks.example").is_err());
}

#[test]
fn oidc_report_checks_metadata_and_keys() {
    let issuer = "https://auth.axiomcore.dev/p/e";
    let discovery = json!({"issuer": issuer, "authorization_endpoint": format!("{issuer}/oauth2/authorize"), "token_endpoint": format!("{issuer}/oauth2/token"),
        "jwks_uri": format!("{issuer}/.well-known/jwks.json"), "userinfo_endpoint": format!("{issuer}/oauth2/userinfo"), "response_types_supported": ["code"],
        "code_challenge_methods_supported": ["S256"], "id_token_signing_alg_values_supported": ["RS256"]});
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let jwks = json!({"axiom_keys_rev": 4, "keys": [
        {"kid": "e1", "kty": "OKP", "crv": "Ed25519", "alg": "EdDSA", "x": b64.encode([1u8; 32])},
        {"kid": "r1", "kty": "RSA", "alg": "RS256", "n": b64.encode([2u8; 256]), "e": "AQAB"}]});
    let epoch = json!({"keys_rev": 4});
    assert_eq!(oidc_report(issuer, &discovery, &jwks, Some(&epoch))["ok"], true);
    let mut weak = jwks.clone();
    weak["keys"][1]["n"] = json!(b64.encode([2u8; 128]));
    assert_eq!(oidc_report(issuer, &discovery, &weak, Some(&epoch))["ok"], false);
    let mut private = jwks.clone();
    private["keys"][0]["d"] = json!("x");
    assert_eq!(oidc_report(issuer, &discovery, &private, Some(&epoch))["ok"], false);
    let mut other = discovery.clone();
    other["token_endpoint"] = json!("https://evil.example/token");
    assert_eq!(oidc_report(issuer, &other, &jwks, Some(&epoch))["ok"], false);
    assert_eq!(oidc_report(issuer, &discovery, &jwks, Some(&json!({"keys_rev": 5})))["ok"], false);
}

const PROVIDER: &str = "3e4f5a6b-7c8d-4e9f-a0b1-2c3d4e5f6a70";
const ISSUER_ID: &str = "4f5a6b7c-8d9e-4fa0-b1c2-3d4e5f6a7b81";

fn code(error: anyhow::Error) -> i32 {
    error.downcast_ref::<Exit>().map(|e| e.code).unwrap_or(-1)
}

#[tokio::test]
async fn providers_remove_resolves_the_selector_and_deletes_by_id() {
    use crate::commands::auth::ProvidersAction;
    let (mock, target) = fixture().await;
    mock.script(&admin_route("GET", "providers"), 200, json!({"providers": [
        {"id": "5a6b7c8d-9e0f-4a1b-8c2d-3e4f5a6b7c92", "kind": "google", "enabled": true, "revision": 2},
        {"id": PROVIDER, "kind": "oidc", "slug": "oidc-okta", "enabled": false, "revision": 4}]}));
    mock.script(&admin_route("DELETE", &format!("providers/{PROVIDER}")), 200, json!({"deleted": true}));
    let remove = ProvidersAction::Remove { provider: "oidc-okta".into(), yes: true };
    super::super::providers_with(&args(), &target, &remove).await.unwrap();
    let seen = mock.seen(&admin_route("DELETE", &format!("providers/{PROVIDER}")));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, Value::Null);
    assert_eq!(seen[0].query, None);
    // By UUID too.
    let by_id = ProvidersAction::Remove { provider: PROVIDER.into(), yes: true };
    super::super::providers_with(&args(), &target, &by_id).await.unwrap();
    assert_eq!(mock.seen(&admin_route("DELETE", &format!("providers/{PROVIDER}"))).len(), 2);
    // An enabled provider is refused locally (the server refuses it too); nothing is deleted.
    let enabled = ProvidersAction::Remove { provider: "google".into(), yes: true };
    assert_eq!(code(super::super::providers_with(&args(), &target, &enabled).await.unwrap_err()), 4);
    // Unknown provider and a bad selector are usage errors.
    let missing = ProvidersAction::Remove { provider: "github".into(), yes: true };
    assert_eq!(code(super::super::providers_with(&args(), &target, &missing).await.unwrap_err()), 2);
    let bad = ProvidersAction::Remove { provider: "oidc-Bad_Slug".into(), yes: true };
    assert_eq!(code(super::super::providers_with(&args(), &target, &bad).await.unwrap_err()), 2);
    assert!(mock.seen(&admin_route("DELETE", "providers/5a6b7c8d-9e0f-4a1b-8c2d-3e4f5a6b7c92")).is_empty());
    // Without --yes and without a terminal nothing is deleted.
    if !std::io::stdin().is_terminal() {
        let unconfirmed = ProvidersAction::Remove { provider: "oidc-okta".into(), yes: false };
        assert_eq!(code(super::super::providers_with(&args(), &target, &unconfirmed).await.unwrap_err()), 2);
        assert_eq!(mock.seen(&admin_route("DELETE", &format!("providers/{PROVIDER}"))).len(), 2);
    }
}

fn issuer_update(id: &str) -> TrustedIssuersAction {
    TrustedIssuersAction::Update {
        id: id.into(),
        audiences: vec![],
        jwks_uri: None,
        algorithms: vec![],
        subject_claim: None,
        email_claim: None,
        no_email_claim: false,
        required_claims: None,
        forbidden_claims: None,
        authorized_parties: vec![],
        clear_authorized_parties: false,
        assurance_mapping: None,
        max_age_seconds: None,
        session_lifetime_seconds: None,
        create_users: false,
        no_create_users: false,
        enable: false,
        disable: false,
    }
}

#[tokio::test]
async fn trusted_issuer_update_reads_the_revision_and_sends_only_given_fields() {
    let (mock, target) = fixture().await;
    let issuer = "https://tenant.example.auth0.com/";
    mock.script(&admin_route("GET", "trusted-issuers"), 200, json!({"trusted_issuers": [{"id": ISSUER_ID, "issuer": issuer, "preset": "auth0", "revision": 6, "enabled": true}]}));
    mock.script(&admin_route("PATCH", &format!("trusted-issuers/{ISSUER_ID}")), 200, json!({"id": ISSUER_ID, "issuer": issuer, "revision": 7, "enabled": true}));
    let mut update = issuer_update(issuer);
    if let TrustedIssuersAction::Update { audiences, algorithms, no_email_claim, required_claims, clear_authorized_parties, assurance_mapping, session_lifetime_seconds, no_create_users, enable, .. } = &mut update {
        *audiences = vec!["api://one".into(), "api://two".into()];
        *algorithms = vec!["RS256".into()];
        *no_email_claim = true;
        *required_claims = Some(r#"{"gty": "password"}"#.into());
        *clear_authorized_parties = true;
        *assurance_mapping = Some("null".into());
        *session_lifetime_seconds = Some(7200);
        *no_create_users = true;
        *enable = true;
    }
    trusted_issuers_with(&args(), &target, &update, None).await.unwrap();
    let seen = mock.seen(&admin_route("PATCH", &format!("trusted-issuers/{ISSUER_ID}")));
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].body, json!({
        "revision": 6, "audiences": ["api://one", "api://two"], "algorithms": ["RS256"], "email_claim": "", "required_claims": {"gty": "password"},
        "authorized_parties": [], "assurance_mapping": null, "session_lifetime_seconds": 7200, "create_users": false, "enabled": true}));
    assert_eq!(mock.seen(&admin_route("GET", "trusted-issuers")).len(), 1);
    // A pure disable by UUID (what a preset issuer accepts with its switch off).
    let mut disable = issuer_update(ISSUER_ID);
    if let TrustedIssuersAction::Update { disable: d, .. } = &mut disable {
        *d = true;
    }
    trusted_issuers_with(&args(), &target, &disable, None).await.unwrap();
    assert_eq!(mock.seen(&admin_route("PATCH", &format!("trusted-issuers/{ISSUER_ID}")))[1].body, json!({"revision": 6, "enabled": false}));
    // An unknown issuer is a usage error and sends no PATCH.
    assert_eq!(code(trusted_issuers_with(&args(), &target, &issuer_update("https://other.example/"), None).await.unwrap_err()), 2);
    assert_eq!(mock.seen(&admin_route("PATCH", &format!("trusted-issuers/{ISSUER_ID}"))).len(), 2);
}

#[test]
fn trusted_issuer_patch_checks_its_flags_locally() {
    assert_eq!(code(trusted_issuer_patch(&issuer_update(ISSUER_ID), Value::Null).unwrap_err()), 2);
    fn with(f: impl FnOnce(&mut TrustedIssuersAction)) -> Result<Value> {
        let mut a = issuer_update(ISSUER_ID);
        f(&mut a);
        trusted_issuer_patch(&a, json!(1))
    }
    assert!(with(|a| if let TrustedIssuersAction::Update { required_claims, .. } = a { *required_claims = Some("[1]".into()) }).is_err());
    assert!(with(|a| if let TrustedIssuersAction::Update { forbidden_claims, .. } = a { *forbidden_claims = Some("{not json".into()) }).is_err());
    assert!(with(|a| if let TrustedIssuersAction::Update { assurance_mapping, .. } = a { *assurance_mapping = Some("[]".into()) }).is_err());
    assert!(with(|a| if let TrustedIssuersAction::Update { jwks_uri, .. } = a { *jwks_uri = Some("http://keys.example/jwks".into()) }).is_err());
    assert!(with(|a| if let TrustedIssuersAction::Update { audiences, .. } = a { *audiences = vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()] }).is_err());
    let mapped = with(|a| {
        if let TrustedIssuersAction::Update { assurance_mapping, forbidden_claims, authorized_parties, max_age_seconds, .. } = a {
            *assurance_mapping = Some(r#"[{"claim": "amr", "contains": "mfa"}]"#.into());
            *forbidden_claims = Some(r#"{"is_anonymous": true}"#.into());
            *authorized_parties = vec!["https://app.example".into()];
            *max_age_seconds = Some(600);
        }
    })
    .unwrap();
    assert_eq!(mapped, json!({"revision": 1, "assurance_mapping": [{"claim": "amr", "contains": "mfa"}], "forbidden_claims": {"is_anonymous": true}, "authorized_parties": ["https://app.example"], "max_token_age_seconds": 600}));
    assert!(issuer_selector(ISSUER_ID).is_ok());
    assert!(issuer_selector("https://issuer.example/").is_ok());
    assert_eq!(code(issuer_selector("my-issuer").unwrap_err()), 2);
    assert_eq!(code(issuer_selector("4F5A6B7C-8D9E-4FA0-B1C2-3D4E5F6A7B81").unwrap_err()), 2);
}

#[tokio::test]
async fn trusted_issuer_remove_deletes_by_id_or_issuer_url() {
    let (mock, target) = fixture().await;
    let issuer = "https://project.supabase.co/auth/v1";
    mock.script(&admin_route("GET", "trusted-issuers"), 200, json!({"trusted_issuers": [{"id": ISSUER_ID, "issuer": issuer, "revision": 2}]}));
    mock.script(&admin_route("DELETE", &format!("trusted-issuers/{ISSUER_ID}")), 200, json!({"deleted": true}));
    trusted_issuers_with(&args(), &target, &TrustedIssuersAction::Remove { id: ISSUER_ID.into(), yes: true }, None).await.unwrap();
    // A UUID needs no lookup.
    assert!(mock.seen(&admin_route("GET", "trusted-issuers")).is_empty());
    trusted_issuers_with(&args(), &target, &TrustedIssuersAction::Remove { id: issuer.into(), yes: true }, None).await.unwrap();
    assert_eq!(mock.seen(&admin_route("GET", "trusted-issuers")).len(), 1);
    let seen = mock.seen(&admin_route("DELETE", &format!("trusted-issuers/{ISSUER_ID}")));
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].body, Value::Null);
    assert_eq!(seen[0].query, None);
    if !std::io::stdin().is_terminal() {
        let unconfirmed = TrustedIssuersAction::Remove { id: ISSUER_ID.into(), yes: false };
        assert_eq!(code(trusted_issuers_with(&args(), &target, &unconfirmed, None).await.unwrap_err()), 2);
        assert_eq!(mock.seen(&admin_route("DELETE", &format!("trusted-issuers/{ISSUER_ID}"))).len(), 2);
    }
}
