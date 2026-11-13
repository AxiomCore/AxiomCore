//! `axiom auth enable|disable` and `axiom auth mfa set` against a loopback stub of the Cloud
//! admin API: both are a settings write (GET settings, merge, PUT settings) with the body rule
//! of the dashboard's `settingsBody`.
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

const PROD: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";

type Log = Arc<Mutex<Vec<(String, String, String)>>>;
type Route = Arc<dyn Fn(&str, &str, &str) -> (u16, String) + Send + Sync>;

/// Serve `route` on 127.0.0.1; every request is recorded as (method, path, body).
fn stub(route: Route) -> (String, Log) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let log: Log = Arc::default();
    let recorded = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let mut parts = line.split_whitespace();
            let method = parts.next().unwrap_or_default().to_string();
            let path = parts.next().unwrap_or_default().to_string();
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" || header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8_lossy(&body).to_string();
            let (status, response) = route(&method, &path, &body);
            recorded.lock().unwrap().push((method, path, body));
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            );
        }
    });
    (address, log)
}

fn axiom(cloud: &str, profile: &Path, args: &[&str]) -> Output {
    std::fs::write(profile.join("auth.json"), r#"{"access_token":"stub-access","refresh_token":"stub-refresh","projects":{}}"#).unwrap();
    let mut full = vec!["auth", "--project", "shop", "--environment", "production"];
    full.extend_from_slice(args);
    let child = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(profile)
        .args(&full)
        .env("CI", "true")
        .env("AXIOM_CLOUD_URL", cloud)
        .env("AXIOM_LOCAL_PROFILE_DIR", profile)
        .env_remove("AXIOM_REFERRAL_CODE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.wait_with_output().unwrap()
}

/// GET settings as the admin service answers it, read-only fields included.
fn settings() -> Value {
    json!({
        "enabled": false, "signup_policy": "invite_only", "access_ttl_seconds": 900, "session_absolute_seconds": 2592000,
        "session_idle_seconds": 604800, "audiences": ["https://auth.axiomcore.dev/p/e/api"], "allowed_origins": ["https://shop.example.com", "https://app.shop.example.com"],
        "revision": 7, "updated_at": "2026-11-01T00:00:00Z", "enabled_methods": ["password"], "require_verified_email": true,
        "password_min_length": 12, "redirect_urls": ["https://shop.example.com/auth/callback"], "logout_urls": [], "default_redirect_url": null,
        "email_sender": "platform", "email_from_name": null, "app_name": "Shop", "invitation_ttl_seconds": 604800, "captcha": "off",
        "turnstile_site_key": null, "turnstile_secret_set": false, "rate_limits": {"signin-ip": 30, "smtp-env-hour": 10},
        "stream_version": 3, "upgrade_pending": false,
        "mfa_policy": "optional", "mfa_factors": ["totp"], "step_up_seconds": 600, "webauthn_rp_id": "shop.example.com",
        "webauthn_rp_name": "Shop", "webauthn_origins": ["https://shop.example.com"], "factor_reset_at": "2026-10-30T00:00:00Z", "hosted_sessions": false
    })
}

/// Keys the dashboard's `settingsBody` sends (SETTINGS_INPUT + SETTINGS_W8_INPUT).
const ACCEPTED: [&str; 28] = [
    "enabled", "signup_policy", "access_ttl_seconds", "session_absolute_seconds", "session_idle_seconds", "audiences", "allowed_origins",
    "enabled_methods", "require_verified_email", "password_min_length", "redirect_urls", "logout_urls", "default_redirect_url", "email_sender",
    "email_from_name", "app_name", "invitation_ttl_seconds", "captcha", "turnstile_site_key", "rate_limits", "revision",
    "mfa_policy", "mfa_factors", "step_up_seconds", "webauthn_rp_id", "webauthn_rp_name", "webauthn_origins", "policy_mode",
];

/// A stub whose GET settings answers `current`; PUT answers `put` (or the merged body when None).
fn cloud(current: Value, put: Option<(u16, String)>, sms: Value) -> (String, Log) {
    let env = format!("/api/v1/projects/shop/auth/environments/{PROD}");
    let route: Route = Arc::new(move |method, path, body| {
        let rest = path.strip_prefix(env.as_str());
        match (method, rest) {
            ("GET", _) if path == "/api/v1/projects/shop/configuration-environments" => {
                (200, format!(r#"[{{"id":"{PROD}","slug":"production","kind":"production"}}]"#))
            }
            ("GET", Some("/settings")) => (200, current.to_string()),
            ("PUT", Some("/settings")) => match &put {
                Some(answer) => answer.clone(),
                None => {
                    let mut saved: Value = serde_json::from_str(body).unwrap();
                    saved["revision"] = json!(saved["revision"].as_i64().unwrap() + 1);
                    (200, saved.to_string())
                }
            },
            ("GET", Some("/overview")) => (200, r#"{"enabled":true,"auth_status":"initializing","stream_version":3,"upgrade_pending":false,"issuer":null,"users_count":null}"#.into()),
            ("GET", Some("/mfa")) => (200, r#"{"policy":"required","factors":["totp","passkey"],"users_with_factors":4,"users_count_capped":false}"#.into()),
            ("GET", Some("/sms")) => (200, sms.to_string()),
            _ => (404, r#"{"error":{"code":"AXAUTH_NOT_FOUND"}}"#.into()),
        }
    });
    stub(route)
}

fn requests(log: &Log) -> Vec<(String, String, String)> {
    log.lock().unwrap().clone()
}

fn puts(log: &Log) -> Vec<Value> {
    requests(log).into_iter().filter(|(m, _, _)| m == "PUT").map(|(_, _, b)| serde_json::from_str(&b).unwrap()).collect()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn enable_reads_settings_and_puts_enabled_with_the_revision_and_only_accepted_keys() {
    let (url, log) = cloud(settings(), None, json!({}));
    let directory = tempfile::tempdir().unwrap();
    let output = axiom(&url, directory.path(), &["enable"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let calls = requests(&log).into_iter().map(|(m, p, _)| format!("{m} {}", p.rsplit('/').next().unwrap())).collect::<Vec<_>>();
    assert_eq!(calls, ["GET configuration-environments", "GET settings", "PUT settings", "GET overview"]);
    let body = &puts(&log)[0];
    assert_eq!(body["enabled"], true);
    assert_eq!(body["revision"], 7);
    let keys = body.as_object().unwrap();
    for key in keys.keys() {
        assert!(ACCEPTED.contains(&key.as_str()), "the PUT carried `{key}`");
    }
    // Every accepted key of the GET goes back unchanged.
    for (key, value) in settings().as_object().unwrap() {
        if ACCEPTED.contains(&key.as_str()) && key != "enabled" {
            assert_eq!(&keys[key], value, "{key}");
        }
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Authentication is turned on. Setup continues in the background."), "{stdout}");
    assert!(stdout.contains("initializing"), "the status is printed: {stdout}");
    assert!(stderr(&output).contains("Signup stays Invite only"), "{}", stderr(&output));

    // --json: a stable object with the status.
    let output = axiom(&url, directory.path(), &["enable", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["enabled"], true);
    assert_eq!(report["changed"], true);
    assert_eq!(report["status"]["auth_status"], "initializing");
}

#[test]
fn enable_when_already_enabled_makes_no_put() {
    let mut current = settings();
    current["enabled"] = json!(true);
    let (url, log) = cloud(current, None, json!({}));
    let directory = tempfile::tempdir().unwrap();
    let output = axiom(&url, directory.path(), &["enable"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(puts(&log).is_empty());
    assert!(String::from_utf8_lossy(&output.stdout).contains("already on"));
}

#[test]
fn disable_needs_yes_without_a_terminal() {
    let mut current = settings();
    current["enabled"] = json!(true);
    let (url, log) = cloud(current, None, json!({}));
    let directory = tempfile::tempdir().unwrap();
    let refused = axiom(&url, directory.path(), &["disable"]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr(&refused));
    assert!(stderr(&refused).contains("--yes"));
    assert!(puts(&log).is_empty());
    let output = axiom(&url, directory.path(), &["disable", "--yes"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let body = &puts(&log)[0];
    assert_eq!(body["enabled"], false);
    assert_eq!(body["revision"], 7);
    assert!(String::from_utf8_lossy(&output.stdout).contains("Authentication is turned off."));
}

#[test]
fn mfa_set_keeps_omitted_values_and_sends_minutes_as_seconds() {
    let (url, log) = cloud(settings(), None, json!({}));
    let directory = tempfile::tempdir().unwrap();
    let output = axiom(&url, directory.path(), &["mfa", "set", "--step-up-minutes", "15"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let body = &puts(&log)[0];
    assert_eq!(body["step_up_seconds"], 900);
    assert_eq!(body["mfa_policy"], "optional");
    assert_eq!(body["mfa_factors"], json!(["totp"]));
    assert_eq!(body["webauthn_rp_id"], "shop.example.com");
    assert_eq!(body["webauthn_rp_name"], "Shop");
    assert_eq!(body["webauthn_origins"], json!(["https://shop.example.com"]));
    assert_eq!(body["revision"], 7);
    assert_eq!(body["enabled"], false);
    for key in body.as_object().unwrap().keys() {
        assert!(ACCEPTED.contains(&key.as_str()), "the PUT carried `{key}`");
    }
    // The resulting view is `mfa status`.
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Policy: required") && stdout.contains("Users with factors: 4"), "{stdout}");

    // Repeatable flags replace the lists; an empty --rp-name clears it.
    let output = axiom(&url, directory.path(), &["mfa", "set", "--factor", "totp", "--factor", "passkey", "--origin", "https://app.shop.example.com", "--rp-name", "", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let body = &puts(&log)[1];
    assert_eq!(body["mfa_factors"], json!(["totp", "passkey"]));
    assert_eq!(body["webauthn_origins"], json!(["https://app.shop.example.com"]));
    assert_eq!(body["webauthn_rp_name"], Value::Null);
    assert_eq!(body["step_up_seconds"], 600);
    let shown: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(shown["policy"], "required");
}

#[test]
fn mfa_set_validation_failures_make_no_put() {
    let mut no_rp = settings();
    no_rp["webauthn_rp_id"] = Value::Null;
    no_rp["webauthn_origins"] = json!([]);
    let cases: [(Value, &[&str], &str); 7] = [
        (settings(), &["mfa", "set", "--step-up-minutes", "1"], "2 to 60 minutes"),
        (settings(), &["mfa", "set", "--step-up-minutes", "61"], "2 to 60 minutes"),
        (no_rp.clone(), &["mfa", "set", "--factor", "passkey"], "need a relying party ID"),
        (settings(), &["mfa", "set", "--origin", "https://other.example.com"], "not an allowed origin"),
        (settings(), &["mfa", "set", "--origin", "https://shop.example.com/path"], "not an exact origin"),
        (settings(), &["mfa", "set", "--rp-id", "auth.axiomcore.dev"], "never an axiomcore.dev name"),
        (no_rp, &["mfa", "set", "--policy", "required", "--factor", "sms"], "Required needs at least one factor"),
    ];
    let directory = tempfile::tempdir().unwrap();
    for (current, args, message) in cases {
        let (url, log) = cloud(current, None, json!({"sms": {"enabled": false, "secret_set": true}}));
        let output = axiom(&url, directory.path(), args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", stderr(&output));
        assert!(stderr(&output).contains(message), "{args:?}: {}", stderr(&output));
        assert!(puts(&log).is_empty(), "{args:?}");
    }
    // SMS with enabled Twilio settings is enrollable, so required is accepted.
    let mut current = settings();
    current["webauthn_rp_id"] = Value::Null;
    current["webauthn_origins"] = json!([]);
    let (url, log) = cloud(current, None, json!({"sms": {"enabled": true, "secret_set": true}}));
    let output = axiom(&url, directory.path(), &["mfa", "set", "--policy", "required", "--factor", "sms"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(puts(&log)[0]["mfa_policy"], "required");
}

#[test]
fn turning_mfa_on_from_off_needs_yes_without_a_terminal() {
    let mut current = settings();
    current["mfa_policy"] = json!("off");
    let (url, log) = cloud(current, None, json!({}));
    let directory = tempfile::tempdir().unwrap();
    let refused = axiom(&url, directory.path(), &["mfa", "set", "--policy", "optional"]);
    assert_eq!(refused.status.code(), Some(2), "{}", stderr(&refused));
    assert!(stderr(&refused).contains("upgrade your app's Axiom sign-in SDK"), "{}", stderr(&refused));
    assert!(puts(&log).is_empty());
    let output = axiom(&url, directory.path(), &["mfa", "set", "--policy", "optional", "--yes"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(puts(&log)[0]["mfa_policy"], "optional");
    // Lowering to off needs no confirmation.
    let (url, log) = cloud(settings(), None, json!({}));
    let output = axiom(&url, directory.path(), &["mfa", "set", "--policy", "off"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(puts(&log)[0]["mfa_policy"], "off");
}

#[test]
fn refused_mfa_writes_map_to_the_dashboard_messages() {
    let directory = tempfile::tempdir().unwrap();
    let (url, _) = cloud(settings(), Some((400, r#"{"error":{"code":"AXAUTH_INVALID_REQUEST"}}"#.into())), json!({}));
    let output = axiom(&url, directory.path(), &["mfa", "set", "--step-up-minutes", "20"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("does not accept these multi-factor settings yet"), "{}", stderr(&output));
    assert!(stderr(&output).contains("Nothing was changed"));
    let (url, _) = cloud(settings(), Some((403, r#"{"error":{"code":"AXAUTH_FEATURE_DISABLED"}}"#.into())), json!({}));
    let output = axiom(&url, directory.path(), &["mfa", "set", "--step-up-minutes", "20"]);
    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(stderr(&output).contains("AXAUTH_FEATURE_DISABLED") && stderr(&output).contains("multi-factor is not available for this environment yet"), "{}", stderr(&output));
    // Other refusals keep the generic mapping.
    let (url, _) = cloud(settings(), Some((409, r#"{"error":{"code":"AXAUTH_CONFLICT"}}"#.into())), json!({}));
    let output = axiom(&url, directory.path(), &["mfa", "set", "--step-up-minutes", "20"]);
    assert_eq!(output.status.code(), Some(4), "{}", stderr(&output));
}

#[test]
fn a_failed_status_read_after_a_saved_change_is_not_a_failure() {
    // The PUT succeeds, then GET overview and GET mfa answer 503: the change is saved, so the
    // command warns and exits 0 (the dashboard keeps its previous view in the same case).
    let env = format!("/api/v1/projects/shop/auth/environments/{PROD}");
    let route: Route = Arc::new(move |method, path, body| match (method, path.strip_prefix(env.as_str())) {
        ("GET", _) if path == "/api/v1/projects/shop/configuration-environments" => {
            (200, format!(r#"[{{"id":"{PROD}","slug":"production","kind":"production"}}]"#))
        }
        ("GET", Some("/settings")) => (200, settings().to_string()),
        ("PUT", Some("/settings")) => (200, body.to_string()),
        _ => (503, r#"{"error":{"code":"AXAUTH_UNAVAILABLE"}}"#.into()),
    });
    let (url, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    let output = axiom(&url, directory.path(), &["enable"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Authentication is turned on"));
    assert!(stderr(&output).contains("the change is saved, but the status could not be read"), "{}", stderr(&output));
    assert_eq!(puts(&log).len(), 1);
    let output = axiom(&url, directory.path(), &["mfa", "set", "--step-up-minutes", "20"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("multi-factor settings are saved, but their status could not be read"), "{}", stderr(&output));
}
