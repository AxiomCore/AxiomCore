use super::*;
use crate::commands::auth::Action;
use crate::commands::cloud_mock::Mock;
use axiom_cloud::CloudClient;

const ENV: &str = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
const CLIENT: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
const CANARY: &str = "SYNTHETIC_CLIENT_SECRET_CANARY";

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

#[test]
fn mfa_input_keeps_omitted_values_and_applies_the_dashboard_rules() {
    let settings = json!({"mfa_policy": "optional", "mfa_factors": ["totp", "recovery_code"], "step_up_seconds": 630, "webauthn_rp_id": " Shop.Example.com ",
        "webauthn_rp_name": "Shop", "webauthn_origins": ["https://shop.example.com"], "allowed_origins": ["https://shop.example.com", "http://localhost:5173"]});
    let kept = mfa_input(&settings, None, &MfaChange::default()).unwrap();
    assert_eq!(kept, json!({"mfa_policy": "optional", "mfa_factors": ["totp"], "step_up_seconds": 630, "webauthn_rp_id": "shop.example.com",
        "webauthn_rp_name": "Shop", "webauthn_origins": ["https://shop.example.com"]}));
    let factors = ["passkey".to_string(), "passkey".to_string(), "totp".to_string()];
    let changed = mfa_input(&settings, None, &MfaChange { policy: Some("required"), factors: Some(&factors), step_up_minutes: Some(2), rp_name: Some("  "), ..Default::default() }).unwrap();
    assert_eq!(changed["mfa_factors"], json!(["passkey", "totp"]));
    assert_eq!(changed["step_up_seconds"], 120);
    assert_eq!(changed["webauthn_rp_name"], Value::Null);
    // Without W8 settings the policy and factors come from GET mfa.
    let mfa = json!({"policy": "required", "factors": [{"type": "totp"}, {"type": "sms"}]});
    let fallback = mfa_input(&json!({}), Some(&mfa), &MfaChange::default()).unwrap();
    assert_eq!((fallback["mfa_policy"].clone(), fallback["mfa_factors"].clone(), fallback["step_up_seconds"].clone()), (json!("required"), json!(["totp", "sms"]), json!(600)));
    assert_eq!(current_policy(&json!({}), None), "off");
    let localhost = ["http://localhost:5173".to_string()];
    let local = mfa_input(&settings, None, &MfaChange { rp_id: Some("localhost"), origins: Some(&localhost), ..Default::default() }).unwrap();
    assert_eq!(local["webauthn_rp_id"], "localhost");

    let problem = |change: MfaChange| mfa_input(&settings, None, &change).unwrap_err().join(" ");
    assert!(problem(MfaChange { step_up_minutes: Some(61), ..Default::default() }).contains("2 to 60 minutes"));
    for bad in ["axiomcore.dev", "auth.axiomcore.dev", "x.axiomcore.dev", "https://shop.example.com", "shop", "-a.example.com"] {
        assert!(problem(MfaChange { rp_id: Some(bad), origins: Some(&[]), ..Default::default() }).contains("never an axiomcore.dev name"), "{bad}");
    }
    let long = "n".repeat(65);
    assert!(problem(MfaChange { rp_name: Some(&long), ..Default::default() }).contains("at most 64 bytes"));
    let nine = (0..9).map(|i| format!("https://a{i}.shop.example.com")).collect::<Vec<_>>();
    assert!(problem(MfaChange { origins: Some(&nine), ..Default::default() }).contains("at most 8"));
    let elsewhere = ["https://other.example.com".to_string()];
    assert!(problem(MfaChange { origins: Some(&elsewhere), ..Default::default() }).contains("not an allowed origin"));
    let outside = ["http://localhost:5173".to_string()];
    assert!(problem(MfaChange { origins: Some(&outside), ..Default::default() }).contains("subdomain"));
    let passkey = ["passkey".to_string()];
    assert!(problem(MfaChange { rp_id: Some(""), origins: Some(&[]), factors: Some(&passkey), ..Default::default() }).contains("need a relying party ID"));
    // With the policy off a passkey factor needs no relying party yet.
    assert!(mfa_input(&settings, None, &MfaChange { policy: Some("off"), rp_id: Some(""), origins: Some(&[]), factors: Some(&passkey), ..Default::default() }).is_ok());
    assert!(problem(MfaChange { rp_id: Some(""), ..Default::default() }).contains("Passkey origins need a relying party ID"));
}

#[test]
fn enrollable_follows_the_dashboard() {
    let all = ["totp".to_string(), "passkey".to_string(), "sms".to_string()];
    assert_eq!(enrollable(&all, None, false), ["totp"]);
    assert_eq!(enrollable(&all[1..], Some("example.com"), true), ["passkey", "sms"]);
    assert!(enrollable(&all[1..], Some(""), false).is_empty());
}

#[test]
fn mfa_refusals_use_the_dashboard_wording() {
    use axiom_cloud::auth_admin::classify;
    let message = |status: u16, body: &str| {
        let error = mfa_refusal(classify(status, body.as_bytes()));
        let exit = error.downcast_ref::<Exit>().unwrap();
        (exit.code, exit.message.clone())
    };
    let (code, text) = message(400, r#"{"error":{"code":"AXAUTH_INVALID_REQUEST"}}"#);
    assert_eq!(code, 2);
    assert!(text.contains("does not accept these multi-factor settings yet"));
    let (code, text) = message(400, "not json");
    assert_eq!(code, 2);
    assert!(text.contains("does not accept"), "{text}");
    let (code, text) = message(403, r#"{"error":{"code":"AXAUTH_FEATURE_DISABLED"}}"#);
    assert_eq!(code, 3);
    assert!(text.starts_with("AXAUTH_FEATURE_DISABLED: multi-factor is not available"));
    let (code, text) = message(409, r#"{"error":{"code":"AXAUTH_CONFLICT"}}"#);
    assert_eq!(code, 4);
    assert!(text.starts_with("AXAUTH_CONFLICT"));
}
