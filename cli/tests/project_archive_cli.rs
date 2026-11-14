//! `axiom project archive`, `restore`, `archive-status` and `list --archived` through the
//! binary, against a loopback stub of the project archive routes.
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

use serde_json::Value;

const PROJECT_ID: &str = "11111111-1111-4111-8111-111111111111";

#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

type Log = Arc<Mutex<Vec<Request>>>;

/// Serve canned responses on 127.0.0.1; every request is recorded with its headers.
fn stub(route: fn(&str, &str) -> (u16, String)) -> (String, Log) {
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
            let mut headers = HashMap::new();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" || header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
                }
            }
            let length = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0usize);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8_lossy(&body).to_string();
            let (status, response) = route(&method, &path);
            recorded.lock().unwrap().push(Request { method, path, headers, body });
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

fn axiom(cloud: &str, profile: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(profile)
        .arg("project")
        .args(args)
        .env("CI", "true")
        .env("AXIOM_CLOUD_URL", cloud)
        .env("AXIOM_LOCAL_PROFILE_DIR", profile)
        .env_remove("AXIOM_REFERRAL_CODE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn view(state: &str) -> String {
    format!(
        r#"{{"enabled":true,"state":"{state}","hidden":true,"confirm":"archive shop","requested_at":"2026-11-14T10:00:00Z","completed_at":null,"databases":[{{"resource_id":"6f1c2a8e-0d4b-4c39-9a51-6a2f3f0b9e11","database_name":"production","status":"ready","operation_id":"","operation_status":"","failure_code":""}}],"blockers":[]}}"#
    )
}

fn route(method: &str, path: &str) -> (u16, String) {
    match (method, path) {
        ("GET", "/api/v1/projects") => (200, format!(r#"[{{"id":"{PROJECT_ID}","slug":"shop","name":"Shop"}}]"#)),
        ("GET", "/api/v1/projects?archived=true") => (
            200,
            format!(r#"[{{"id":"{PROJECT_ID}","slug":"shop","name":"Shop","archive":{{"state":"archived","requested_at":"2026-11-14T10:00:00Z"}}}}]"#),
        ),
        ("POST", "/api/v1/projects/shop/archive") => (202, view("archiving")),
        ("POST", "/api/v1/projects/shop/restore") => (202, view("restoring")),
        ("GET", "/api/v1/projects/shop/archive") => (200, view("archived")),
        _ => (404, r#"{"code":"AXRESOURCE_INVALID"}"#.into()),
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}
fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn archive_without_a_terminal_needs_yes_and_sends_nothing() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    for args in [&["archive", "shop"][..], &["archive", "--project", "shop", "--wait"], &["archive"]] {
        let output = axiom(&cloud, directory.path(), args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {}", stderr(&output));
        assert_eq!(stderr(&output), "axiom project archive: pass --yes to confirm without a prompt\n");
        assert!(stdout(&output).is_empty());
    }
    assert!(log.lock().unwrap().is_empty(), "no request without confirmation: {:?}", log.lock().unwrap());
}

#[test]
fn archive_with_yes_posts_the_confirm_phrase_with_an_idempotency_key() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["archive", "Shop", "--yes"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).starts_with("Archive of shop started"), "{}", stdout(&output));
    assert!(stdout(&output).contains("`axiom project archive-status shop`"));
    let requests = log.lock().unwrap().clone();
    let posts: Vec<_> = requests.iter().filter(|r| r.method == "POST").collect();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/api/v1/projects/shop/archive");
    assert_eq!(serde_json::from_str::<Value>(&posts[0].body).unwrap(), serde_json::json!({"confirm": "archive shop"}));
    assert_eq!(posts[0].headers["authorization"], "Bearer stub-access");
    uuid::Uuid::parse_str(&posts[0].headers["idempotency-key"]).unwrap();

    let output = axiom(&cloud, directory.path(), &["archive", "--project", "shop", "--yes", "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let value: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["state"], "archiving");
}

#[test]
fn restore_status_and_archived_list_need_no_confirmation() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["restore", "shop"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).starts_with("Restore of shop started"), "{}", stdout(&output));
    let posts: Vec<_> = log.lock().unwrap().iter().filter(|r| r.method == "POST").cloned().collect();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, "/api/v1/projects/shop/restore");
    assert_eq!(posts[0].body, "{}");
    uuid::Uuid::parse_str(&posts[0].headers["idempotency-key"]).unwrap();

    let output = axiom(&cloud, directory.path(), &["archive-status", "shop"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).starts_with("Project shop: archived"), "{}", stdout(&output));

    let output = axiom(&cloud, directory.path(), &["list", "--archived"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("Shop  shop  (archived, since 2026-11-14T10:00:00Z)"), "{}", stdout(&output));
    assert!(log.lock().unwrap().iter().any(|r| r.path == "/api/v1/projects?archived=true"));
}

fn failing_route(method: &str, path: &str) -> (u16, String) {
    match (method, path) {
        ("GET", "/api/v1/projects/shop/archive") => (
            200,
            view("failed").replace(r#""operation_status":"","failure_code":"""#, r#""operation_status":"failed","failure_code":"AXRESOURCE_SNAPSHOT_UNVERIFIED""#),
        ),
        _ => route(method, path),
    }
}

#[test]
fn json_wait_that_ends_failed_still_prints_the_last_view() {
    let (cloud, _log) = stub(failing_route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["archive", "shop", "--yes", "--wait", "--json"]);
    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let value: Value = serde_json::from_str(&stdout(&output)).unwrap_or_else(|e| panic!("{e}: {}", stdout(&output)));
    assert_eq!(value["state"], "failed");
    assert_eq!(value["databases"][0]["failure_code"], "AXRESOURCE_SNAPSHOT_UNVERIFIED");
    let text = stderr(&output);
    assert!(text.contains("The archive of shop failed") && text.contains("  - production (AXRESOURCE_SNAPSHOT_UNVERIFIED)"), "{text}");
}
