//! `axiom auth` against a loopback stub of the Cloud admin API (auth-surfaces/v1 W4.5).
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

const DEV: &str = "0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c";
const PROD: &str = "1c2d3e4f-5a6b-4c7d-8e9f-0a1b2c3d4e5f";
const USER: &str = "7d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
const SHOP_ID: &str = "6a0c1f8e-3b2d-4c5e-9f70-1a2b3c4d5e6f";
/// A credential-shaped canary that must never be echoed by template output.
const TOKEN: &str = "axr1_SYNTHETICTEMPLATECANARY0123";

/// `GET /api/v1/projects`: "storefront" is a lowercase display name whose slug differs,
/// and two projects share the name "twin".
fn projects() -> String {
    let project = |id: &str, name: &str, slug: &str| format!(r#"{{"id":"{id}","name":"{name}","slug":"{slug}","visibility":"private","created_at":"","updated_at":""}}"#);
    format!(
        "[{},{},{},{}]",
        project(SHOP_ID, "Web Shop", "shop"),
        project("2b3c4d5e-6f70-4a1b-8c2d-3e4f5a6b7c8d", "storefront", "shop-eu"),
        project("3c4d5e6f-7081-4b2c-9d3e-4f5a6b7c8d9e", "twin", "twin-a"),
        project("4d5e6f70-8192-4c3d-8e4f-5a6b7c8d9e0f", "Twin", "twin-b")
    )
}

type Log = Arc<Mutex<Vec<(String, String, String)>>>;

/// Serve canned responses on 127.0.0.1; `route` maps (method, path with
/// query) to (status, body). Every request is recorded.
fn stub(route: fn(&str, &str, &str) -> (u16, String)) -> (String, Log) {
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

fn profile(directory: &Path) {
    std::fs::write(
        directory.join("auth.json"),
        r#"{"access_token":"stub-access","refresh_token":"stub-refresh","projects":{}}"#,
    )
    .unwrap();
}

fn axiom(cloud: &str, profile: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(profile)
        .args(args)
        .env("CI", "true")
        .env("AXIOM_CLOUD_URL", cloud)
        .env("AXIOM_LOCAL_PROFILE_DIR", profile)
        .env_remove("AXIOM_REFERRAL_CODE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(input) = stdin {
        child.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn route(method: &str, path: &str, body: &str) -> (u16, String) {
    let base = "/api/v1/projects/shop";
    let env = |e: &str| format!("{base}/auth/environments/{e}");
    match (method, path) {
        ("GET", "/api/v1/projects") => (200, projects()),
        ("GET", p) if p.ends_with("/configuration-environments") && p.starts_with("/api/v1/projects/shop") => (
            200,
            format!(r#"[{{"id":"{DEV}","slug":"development","kind":"development"}},{{"id":"{PROD}","slug":"production","kind":"production"}}]"#),
        ),
        ("GET", p) if p.starts_with(&format!("{}/users?", env(PROD))) => (
            200,
            format!(r#"{{"users":[{{"id":"{USER}","kind":"password","email":"jane@example.com","display_name":null,"status":"active","email_verified":true,"providers":["google"],"created_at":"2026-10-26T00:00:00Z","last_sign_in_at":null,"active_sessions":1,"roles_count":0}}],"next_cursor":null}}"#),
        ),
        ("POST", p) if p == format!("{}/users/search", env(PROD)) => (200, r#"{"users":[],"next_cursor":null}"#.into()),
        ("POST", p) if p == format!("{}/users/{USER}/ban", env(PROD)) => (409, r#"{"error":{"code":"AXAUTH_CONFLICT"}}"#.into()),
        ("GET", p) if p == format!("{}/settings", env(DEV)) => (403, r#"{"error":{"code":"AXAUTH_FORBIDDEN"}}"#.into()),
        ("GET", p) if p == format!("{}/providers", env(DEV)) => (200, r#"{"providers":[]}"#.into()),
        ("GET", "/api/v1/projects/shop-eu/auth/environments/0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c/email/usage") => (200, r#"{"sent":1,"budget":10,"suppressions":{}}"#.into()),
        ("POST", p) if p == format!("{}/email/suppressions/remove", env(DEV)) => (200, r#"{"removed":true}"#.into()),
        ("GET", p) if p == format!("{}/templates/signup", env(DEV)) => (
            200,
            format!(r#"{{"kind":"signup","overridden":true,"subject":"Code {TOKEN}","intro":"Use {TOKEN} now","outro":null,"revision":2,"defaults":{{"outro":"Bye"}}}}"#),
        ),
        ("GET", p) if p == format!("{}/templates", env(DEV)) => (200, format!(r#"{{"templates":[{{"kind":"signup","overridden":true,"subject":"Code {TOKEN}","revision":2}}]}}"#)),
        ("POST", p) if p == format!("{}/templates/signup/preview", env(DEV)) => (
            200,
            format!(r#"{{"subject":"Code {TOKEN}","text":"Use {TOKEN} now: https://auth.axiomcore.dev/{SHOP_ID}/{DEV}/sign-in/link#axiom_link=preview","html":"<p>Use {TOKEN} now</p>"}}"#),
        ),
        ("PUT", p) if p == format!("{}/templates/signup", env(DEV)) => {
            let mut saved: serde_json::Value = serde_json::from_str(body).unwrap();
            saved["kind"] = "signup".into();
            saved["overridden"] = true.into();
            saved["revision"] = 3.into();
            (200, saved.to_string())
        }
        ("POST", p) if p == format!("{}/providers", env(DEV)) => {
            let request: serde_json::Value = serde_json::from_str(body).unwrap();
            (201, format!(r#"{{"id":"{USER}","kind":"google","client_id":{},"client_secret_set":true,"enabled":false,"revision":1}}"#, request["client_id"]))
        }
        _ => (404, r#"{"error":{"code":"AXAUTH_NOT_FOUND"}}"#.into()),
    }
}

#[test]
fn users_list_resolves_the_environment_and_prints_golden_masked_json() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", "production", "users", "list", "--status", "active", "--limit", "10", "--json"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let golden = format!(
        "{{\n  \"next_cursor\": null,\n  \"users\": [\n    {{\n      \"active_sessions\": 1,\n      \"created_at\": \"2026-10-26T00:00:00Z\",\n      \"display_name\": null,\n      \"email\": \"j***@e***.com\",\n      \"email_verified\": true,\n      \"id\": \"{USER}\",\n      \"kind\": \"password\",\n      \"last_sign_in_at\": null,\n      \"providers\": [\n        \"google\"\n      ],\n      \"roles_count\": 0,\n      \"status\": \"active\"\n    }}\n  ]\n}}\n"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), golden);
    let requests = log.lock().unwrap().clone();
    assert_eq!(requests[0].1, "/api/v1/projects");
    assert_eq!(requests[1].1, "/api/v1/projects/shop/configuration-environments");
    assert_eq!(requests[2].1, format!("/api/v1/projects/shop/auth/environments/{PROD}/users?status=active&limit=10"));
    // An email search never puts the address in a URL.
    let output = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", PROD, "users", "list", "--search", "jane@"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let requests = log.lock().unwrap().clone();
    let search = requests.last().unwrap();
    assert!(search.1.ends_with("/users/search") && !search.1.contains("jane"));
    assert!(search.2.contains("\"email_prefix\":\"jane@\""));
    let revealed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", "production", "users", "list", "--limit", "5", "--reveal-email"], None);
    assert!(String::from_utf8_lossy(&revealed.stdout).contains("jane@example.com"));
}

#[test]
fn forbidden_and_conflict_map_to_exit_codes_3_and_4_and_usage_to_2() {
    let (cloud, _) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let forbidden = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "settings", "get"], None);
    assert_eq!(forbidden.status.code(), Some(3), "{}", String::from_utf8_lossy(&forbidden.stderr));
    assert!(String::from_utf8_lossy(&forbidden.stderr).contains("AXAUTH_FORBIDDEN"));
    let conflict = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", "production", "users", "ban", USER, "--reason", "abuse"], None);
    assert_eq!(conflict.status.code(), Some(4), "{}", String::from_utf8_lossy(&conflict.stderr));
    let unknown = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", "staging", "status"], None);
    assert_eq!(unknown.status.code(), Some(2));
    let unconfirmed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "users", "revoke", USER], None);
    assert_eq!(unconfirmed.status.code(), Some(2));
    let flag = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "providers", "set", "google", "--client-secret", "x"], None);
    assert_eq!(flag.status.code(), Some(2));
}

#[test]
fn provider_secrets_come_from_stdin_and_are_never_printed() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(
        &cloud,
        directory.path(),
        &["auth", "--project", "shop", "providers", "set", "google", "--client-id", "client.apps", "--client-secret-stdin", "--json"],
        Some("SYNTHETIC_PROVIDER_SECRET\n"),
    );
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stdout.contains("SYNTHETIC_PROVIDER_SECRET") && !stderr.contains("SYNTHETIC_PROVIDER_SECRET"));
    assert!(stdout.contains("\"client_secret_set\": true"));
    let requests = log.lock().unwrap().clone();
    let create = requests.iter().find(|(m, p, _)| m == "POST" && p.ends_with("/providers")).unwrap();
    let body: serde_json::Value = serde_json::from_str(&create.2).unwrap();
    assert_eq!(body["client_secret"], "SYNTHETIC_PROVIDER_SECRET");
    assert_eq!(body["enabled"], false);
    // No prompt is possible without a terminal and no secret was piped.
    let refused = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "providers", "set", "google", "--client-id", "x"], None);
    assert_eq!(refused.status.code(), Some(2));
}

fn admin_requests(log: &Log) -> Vec<(String, String, String)> {
    log.lock().unwrap().iter().filter(|(_, p, _)| p.contains("/auth/environments/")).cloned().collect()
}

#[test]
fn projects_resolve_by_name_slug_or_id_through_the_project_list() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    // A lowercase display name that differs from its slug resolves to that slug.
    let named = axiom(&cloud, directory.path(), &["auth", "--project", "storefront", "--environment", DEV, "email", "usage", "--json"], None);
    assert!(named.status.success(), "{}", String::from_utf8_lossy(&named.stderr));
    assert_eq!(admin_requests(&log).last().unwrap().1, format!("/api/v1/projects/shop-eu/auth/environments/{DEV}/email/usage"));
    // Slugs and ids still resolve.
    for given in ["shop", SHOP_ID, "web shop"] {
        let output = axiom(&cloud, directory.path(), &["auth", "--project", given, "--environment", "production", "users", "list", "--limit", "1"], None);
        assert!(output.status.success(), "{given}: {}", String::from_utf8_lossy(&output.stderr));
        assert!(admin_requests(&log).last().unwrap().1.starts_with("/api/v1/projects/shop/"), "{given}");
    }
    // A name two projects share, and an unknown slug-shaped value: exit 2, nothing sent to a project.
    let before = log.lock().unwrap().len();
    let ambiguous = axiom(&cloud, directory.path(), &["auth", "--project", "twin", "--environment", DEV, "email", "usage"], None);
    assert_eq!(ambiguous.status.code(), Some(2), "{}", String::from_utf8_lossy(&ambiguous.stderr));
    assert!(String::from_utf8_lossy(&ambiguous.stderr).contains("more than one project"));
    let unknown = axiom(&cloud, directory.path(), &["auth", "--project", "no-such-project", "--environment", DEV, "email", "usage"], None);
    assert_eq!(unknown.status.code(), Some(2), "{}", String::from_utf8_lossy(&unknown.stderr));
    let after: Vec<_> = log.lock().unwrap()[before..].to_vec();
    assert!(after.iter().all(|(m, p, _)| m == "GET" && p == "/api/v1/projects"), "{after:?}");
}

#[test]
fn email_unsuppress_needs_yes_without_a_terminal() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let unconfirmed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "email", "unsuppress", "bounced@example.com"], None);
    assert_eq!(unconfirmed.status.code(), Some(2), "{}", String::from_utf8_lossy(&unconfirmed.stderr));
    assert!(String::from_utf8_lossy(&unconfirmed.stderr).contains("--yes"));
    assert!(!log.lock().unwrap().iter().any(|(m, p, _)| m == "POST" && p.ends_with("/email/suppressions/remove")));
    let confirmed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "email", "unsuppress", "bounced@example.com", "--yes"], None);
    assert!(confirmed.status.success(), "{}", String::from_utf8_lossy(&confirmed.stderr));
    let requests = log.lock().unwrap().clone();
    let removed: Vec<_> = requests.iter().filter(|(m, p, _)| m == "POST" && p.ends_with("/email/suppressions/remove")).collect();
    assert_eq!(removed.len(), 1);
    assert_eq!(removed[0].2, r#"{"email":"bounced@example.com"}"#);
}

#[test]
fn template_text_comes_from_files_or_stdin_and_tokens_are_redacted_in_output() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    // Template text is never argv.
    let argv = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "templates", "set", "signup", "--subject", "Welcome"], None);
    assert_eq!(argv.status.code(), Some(2), "{}", String::from_utf8_lossy(&argv.stderr));
    assert!(!log.lock().unwrap().iter().any(|(m, _, _)| m == "PUT"));

    // Shown text: token-like values are redacted in human and JSON output of every
    // templates command, unless --reveal-text.
    for command in [&["show", "signup"][..], &["list"], &["preview", "signup"], &["preview", "signup", "--html"]] {
        for json in [false, true] {
            let mut args = vec!["auth", "--project", "shop", "--environment", DEV, "templates"];
            args.extend_from_slice(command);
            if json {
                args.push("--json");
            }
            let shown = axiom(&cloud, directory.path(), &args, None);
            assert!(shown.status.success(), "{command:?}: {}", String::from_utf8_lossy(&shown.stderr));
            let stdout = String::from_utf8_lossy(&shown.stdout);
            assert!(!stdout.contains(TOKEN) && !stdout.contains("axr1_"), "{command:?}: {stdout}");
            assert!(stdout.contains("[redacted]"), "{command:?}: {stdout}");
            if command == ["preview", "signup"] {
                // The sample sign-in link is kept.
                assert!(stdout.contains(&format!("/{SHOP_ID}/{DEV}/sign-in/link#axiom_link=preview")), "{stdout}");
            }
        }
    }
    let revealed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "templates", "show", "signup", "--reveal-text"], None);
    assert!(String::from_utf8_lossy(&revealed.stdout).contains(TOKEN));
    let revealed = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "templates", "preview", "signup", "--reveal-text"], None);
    assert!(String::from_utf8_lossy(&revealed.stdout).contains(TOKEN));

    // Empty stdin is refused rather than resetting the subject to the default.
    let empty = axiom(&cloud, directory.path(), &["auth", "--project", "shop", "--environment", DEV, "templates", "set", "signup", "--subject-file", "-"], Some(""));
    assert_eq!(empty.status.code(), Some(2), "{}", String::from_utf8_lossy(&empty.stderr));
    assert!(!log.lock().unwrap().iter().any(|(m, _, _)| m == "PUT"));

    // The subject from stdin is sent as given; the saved template echoed back is redacted.
    let set = axiom(
        &cloud,
        directory.path(),
        &["auth", "--project", "shop", "--environment", DEV, "templates", "set", "signup", "--subject-file", "-", "--json"],
        Some(&format!("Welcome {TOKEN}\n")),
    );
    assert!(set.status.success(), "{}", String::from_utf8_lossy(&set.stderr));
    let stdout = String::from_utf8_lossy(&set.stdout);
    assert!(!stdout.contains(TOKEN), "{stdout}");
    let requests = log.lock().unwrap().clone();
    let put = requests.iter().find(|(m, _, _)| m == "PUT").unwrap();
    let body: serde_json::Value = serde_json::from_str(&put.2).unwrap();
    assert_eq!(body["subject"], format!("Welcome {TOKEN}"));
    assert_eq!(body["revision"], 2);
}
