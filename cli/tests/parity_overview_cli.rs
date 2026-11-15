//! Dashboard overview and restore-preview reads through the CLI against a loopback API.
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

use serde_json::Value;

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
            let length = headers
                .get("content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0usize);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let body = String::from_utf8_lossy(&body).to_string();
            let (status, response) = route(&method, &path);
            recorded.lock().unwrap().push(Request {
                method,
                path,
                headers,
                body,
            });
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
    let child = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .current_dir(profile)
        .args(args)
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

fn route(_: &str, _: &str) -> (u16, String) {
    (
        200,
        r#"{"branches":[],"undone_migrations":[],"undone_total":0}"#.into(),
    )
}

#[test]
fn branch_overview_uses_dashboard_read_with_optional_environment() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    for suffix in [vec![], vec!["--environment", "production"]] {
        let mut args = vec!["branch", "overview", "--project", "shop", "--json"];
        args.extend(suffix);
        let output = axiom(&cloud, directory.path(), &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _: Value = serde_json::from_slice(&output.stdout).unwrap();
    }
    let requests = log.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].path, "/api/v1/projects/shop/branches/overview");
    assert_eq!(
        requests[1].path,
        "/api/v1/projects/shop/branches/overview?environment=production"
    );
    for request in requests.iter() {
        assert_eq!(request.method, "GET");
        assert!(request.body.is_empty());
        assert_eq!(request.headers["authorization"], "Bearer stub-access");
    }
}

#[test]
fn recovery_preview_uses_backup_or_point_in_time_query() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let resource = "6f1c2a8e-0d4b-4c39-9a51-6a2f3f0b9e11";
    for selector in [
        vec!["--backup", "backup-1"],
        vec!["--at", "2026-11-14T10:00:00Z"],
    ] {
        let mut args = vec![
            "project",
            "resources",
            "--project",
            "shop",
            "recovery-preview",
            "--resource",
            resource,
            "--json",
        ];
        args.extend(selector);
        let output = axiom(&cloud, directory.path(), &args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let _: Value = serde_json::from_slice(&output.stdout).unwrap();
    }
    let requests = log.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].path,
        format!("/api/v1/projects/shop/resources/{resource}/recovery/preview?backup_id=backup-1")
    );
    assert_eq!(requests[1].path, format!("/api/v1/projects/shop/resources/{resource}/recovery/preview?at=2026-11-14T10%3A00%3A00Z"));
    for request in requests.iter() {
        assert_eq!(request.method, "GET");
        assert!(request.body.is_empty());
    }
}

#[test]
fn recovery_preview_invalid_selectors_send_nothing() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    for selector in [
        vec![],
        vec!["--backup", "one", "--at", "2026-11-14T10:00:00Z"],
        vec!["--at", "invalid"],
        vec!["--backup", " "],
    ] {
        let mut args = vec![
            "project",
            "resources",
            "--project",
            "shop",
            "recovery-preview",
            "--resource",
            "6f1c2a8e-0d4b-4c39-9a51-6a2f3f0b9e11",
        ];
        args.extend(selector);
        assert!(!axiom(&cloud, directory.path(), &args).status.success());
    }
    assert!(log.lock().unwrap().is_empty());
}
