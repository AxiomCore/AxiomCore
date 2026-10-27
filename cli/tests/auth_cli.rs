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
        ("GET", p) if p == format!("{base}/configuration-environments") => (
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
    assert_eq!(requests[0].1, "/api/v1/projects/shop/configuration-environments");
    assert_eq!(requests[1].1, format!("/api/v1/projects/shop/auth/environments/{PROD}/users?status=active&limit=10"));
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
