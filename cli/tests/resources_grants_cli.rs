//! `axiom project resources grants`, `grant` and `revoke` (runtime-data/v1 database
//! access) against a loopback stub of the resources and secret-workloads routes.
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

use serde_json::Value;

const RESOURCE: &str = "3f1d2c4b-5a6e-4f70-8a9b-0c1d2e3f4a5b";
const ENVIRONMENT: &str = "9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d";
const ACTIVE: &str = "11111111-2222-4333-8444-555555555555";
const DISABLED: &str = "22222222-3333-4444-8555-666666666666";
const SECRETS_ONLY: &str = "33333333-4444-4555-8666-777777777777";
const MISSING: &str = "44444444-5555-4666-8777-888888888888";
const GRANT: &str = "55555555-6666-4777-8888-999999999999";
const PLAN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const AUDIENCE: &str = "https://api.axiomcore.dev/runtime/v1/database-leases";

#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

type Log = Arc<Mutex<Vec<Request>>>;

/// Serve canned responses on 127.0.0.1; every request is recorded with its headers.
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
            let (status, response) = route(&method, &path, &body);
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
    let mut command = Command::new(env!("CARGO_BIN_EXE_axiom-cli"));
    command
        .current_dir(profile)
        .args(["project", "resources", "--project", "shop"])
        .args(args)
        .env("CI", "true")
        .env("AXIOM_CLOUD_URL", cloud)
        .env("AXIOM_LOCAL_PROFILE_DIR", profile)
        .env_remove("AXIOM_REFERRAL_CODE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.output().unwrap()
}

fn grant_json(requirement: &str, workload: &str, expires_at: &str, revoked_at: Option<&str>) -> String {
    format!(
        r#"{{"id":"{GRANT}","resource_id":"{RESOURCE}","environment_id":"{ENVIRONMENT}","requirement":"{requirement}","purpose":"app","workload_id":"{workload}","plan_sha256":"{PLAN}","consumer":"server","expires_at":"{expires_at}","revoked_at":{},"created_at":"2026-11-01T00:00:00Z","revision":1}}"#,
        revoked_at.map(|v| format!("\"{v}\"")).unwrap_or_else(|| "null".into())
    )
}

fn route(method: &str, path: &str, body: &str) -> (u16, String) {
    let resources = "/api/v1/projects/shop/resources";
    let grants = format!("{resources}/{RESOURCE}/database-grants");
    match (method, path) {
        ("GET", p) if p == resources => (
            200,
            format!(r#"{{"can_manage":true,"resources":[{{"id":"{RESOURCE}","environment":"production","environment_id":"{ENVIRONMENT}","database":"sqlite","engine":"libsql","status":"ready","generation":3}}],"operations":[]}}"#),
        ),
        ("GET", p) if p == format!("/api/v1/projects/shop/configuration-environments/{ENVIRONMENT}/secret-workloads") => (
            200,
            format!(
                r#"[{{"id":"{ACTIVE}","issuer":"https://accounts.google.com","subject":"104200000000000000001","audience":"{AUDIENCE}","plan_sha256":"{PLAN}","deployment_id":null,"status":"active"}},
                    {{"id":"{DISABLED}","issuer":"https://accounts.google.com","subject":"104200000000000000002","audience":"{AUDIENCE}","plan_sha256":"{PLAN}","deployment_id":null,"status":"disabled"}},
                    {{"id":"{SECRETS_ONLY}","issuer":"https://accounts.google.com","subject":"104200000000000000003","audience":"https://api.axiomcore.dev/runtime/v1/secret-leases","plan_sha256":"{PLAN}","deployment_id":null,"status":"active"}}]"#
            ),
        ),
        ("GET", p) if p == grants => (
            200,
            format!(
                r#"{{"grants":[{},{},{},{}]}}"#,
                grant_json("tasks", ACTIVE, "2099-01-01T00:00:00Z", None),
                grant_json("orders", ACTIVE, "2020-01-01T00:00:00Z", None),
                grant_json("audit", DISABLED, "2099-01-01T00:00:00Z", Some("2026-11-02T00:00:00Z")),
                grant_json("legacy", MISSING, "2099-01-01T00:00:00Z", None),
            ),
        ),
        ("POST", p) if p == grants => {
            let request: Value = serde_json::from_str(body).unwrap();
            match request["requirement"].as_str().unwrap() {
                "taken" => (409, r#"{"error":{"code":"AXRESOURCE_CONFLICT","message":"The request conflicts with the current state."}}"#.into()),
                "flaky" => (503, r#"{"error":{"code":"AXRESOURCE_UNAVAILABLE","message":"The service is unavailable."}}"#.into()),
                requirement => (
                    201,
                    format!(r#"{{"grant":{}}}"#, grant_json(requirement, request["workload_id"].as_str().unwrap(), request["expires_at"].as_str().unwrap(), None)),
                ),
            }
        }
        ("POST", p) if p == format!("{grants}/{GRANT}/revoke") => {
            (200, format!(r#"{{"grant":{}}}"#, grant_json("tasks", ACTIVE, "2099-01-01T00:00:00Z", Some("2026-11-13T00:00:00Z"))))
        }
        ("POST", p) if p == format!("{grants}/{MISSING}/revoke") => {
            (409, r#"{"error":{"code":"AXRESOURCE_CONFLICT","message":"The request conflicts with the current state."}}"#.into())
        }
        _ => (404, r#"{"error":{"code":"AXRESOURCE_NOT_FOUND"}}"#.into()),
    }
}

fn posts(log: &Log) -> Vec<Request> {
    log.lock().unwrap().iter().filter(|r| r.method == "POST").cloned().collect()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn grants_lists_requirement_workload_subject_state_and_id() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["grants", "--environment", "production"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    let line = |requirement: &str| text.lines().find(|l| l.starts_with(requirement)).unwrap_or_else(|| panic!("no {requirement} row in\n{text}")).to_string();
    assert!(text.lines().next().unwrap().starts_with("REQUIREMENT"), "{text}");
    let tasks = line("tasks");
    assert!(tasks.contains("104200000000000000001") && tasks.contains("Active") && tasks.contains(GRANT) && tasks.contains("2099-01-01"), "{tasks}");
    assert!(line("orders").contains("Expired"));
    let audit = line("audit");
    assert!(audit.contains("Revoked") && audit.contains("104200000000000000002 (disabled)"), "{audit}");
    // A workload outside the environment's list is shown by its ID.
    assert!(line("legacy").contains(MISSING));
    let requests = log.lock().unwrap().clone();
    let paths: Vec<_> = requests.iter().map(|r| r.path.as_str()).collect();
    assert!(paths.contains(&format!("/api/v1/projects/shop/resources/{RESOURCE}/database-grants").as_str()), "{paths:?}");
    assert!(paths.contains(&format!("/api/v1/projects/shop/configuration-environments/{ENVIRONMENT}/secret-workloads").as_str()), "{paths:?}");
    assert!(requests.iter().all(|r| r.method == "GET" && r.headers.get("authorization").map(String::as_str) == Some("Bearer stub-access")));

    let json = axiom(&cloud, directory.path(), &["grants", "--resource", RESOURCE, "--json"]);
    assert!(json.status.success(), "{}", stderr(&json));
    let value: Value = serde_json::from_str(&stdout(&json)).unwrap();
    assert_eq!(value["grants"].as_array().unwrap().len(), 4);
    assert_eq!(value["grants"][0]["state"], "active");
    assert_eq!(value["grants"][1]["state"], "expired");
    assert_eq!(value["grants"][2]["state"], "revoked");
    assert_eq!(value["grants"][0]["workload_subject"], "104200000000000000001");
    assert!(value["grants"][3]["workload_subject"].is_null());
}

#[test]
fn grant_by_environment_sends_the_dashboard_body_with_an_idempotency_key() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let before = chrono::Utc::now();
    let output = axiom(&cloud, directory.path(), &["grant", "--environment", "production", "--requirement", "tasks", "--workload", "104200000000000000001", "--expires-days", "7"]);
    let after = chrono::Utc::now();
    assert!(output.status.success(), "{}", stderr(&output));
    let posts = posts(&log);
    assert_eq!(posts.len(), 1);
    let post = &posts[0];
    assert_eq!(post.path, format!("/api/v1/projects/shop/resources/{RESOURCE}/database-grants"));
    let key = post.headers.get("idempotency-key").expect("Idempotency-Key header");
    uuid::Uuid::parse_str(key).unwrap();
    let body: Value = serde_json::from_str(&post.body).unwrap();
    let keys: Vec<_> = body.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys.len(), 6, "{body}");
    assert_eq!(body["requirement"], "tasks");
    assert_eq!(body["purpose"], "app");
    assert_eq!(body["workload_id"], ACTIVE);
    assert_eq!(body["plan_sha256"], PLAN);
    assert_eq!(body["consumer"], "server");
    let expires = body["expires_at"].as_str().unwrap();
    assert!(expires.ends_with('Z') && expires.len() == 24, "{expires}");
    let expires = chrono::DateTime::parse_from_rfc3339(expires).unwrap();
    let week = chrono::Duration::days(7);
    assert!(expires >= before + week - chrono::Duration::seconds(1) && expires <= after + week, "{expires}");
    let text = stdout(&output);
    assert!(text.starts_with("Database access granted to 104200000000000000001 for tasks in production until "), "{text}");
    assert!(stderr(&output).contains(key.as_str()));

    // 90 days stays five minutes inside the API's limit; the default is 30 days.
    let ninety = axiom(&cloud, directory.path(), &["grant", "--resource", RESOURCE, "--requirement", "jobs", "--workload", ACTIVE, "--expires-days", "90"]);
    assert!(ninety.status.success(), "{}", stderr(&ninety));
    let thirty = axiom(&cloud, directory.path(), &["grant", "--resource", RESOURCE, "--requirement", "mail", "--workload", ACTIVE]);
    assert!(thirty.status.success(), "{}", stderr(&thirty));
    let posts = self::posts(&log);
    let at = |i: usize| chrono::DateTime::parse_from_rfc3339(serde_json::from_str::<Value>(&posts[i].body).unwrap()["expires_at"].as_str().unwrap()).unwrap();
    let now = chrono::Utc::now();
    assert!(at(1) <= now + chrono::Duration::days(90) - chrono::Duration::minutes(5));
    assert!(at(1) >= now + chrono::Duration::days(90) - chrono::Duration::minutes(6));
    assert!(at(2) <= now + chrono::Duration::days(30) && at(2) >= now + chrono::Duration::days(30) - chrono::Duration::minutes(1));
    assert_ne!(posts[1].headers["idempotency-key"], posts[2].headers["idempotency-key"]);
}

#[test]
fn grant_refuses_bad_input_and_ineligible_workloads_before_any_write() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let cases: [(&[&str], &str); 7] = [
        (&["--requirement", "Tasks", "--workload", ACTIVE], "Enter the requirement name: a lowercase letter, then up to 63 letters or digits."),
        (&["--requirement", "task-s", "--workload", ACTIVE], "Enter the requirement name"),
        (&["--requirement", "tasks", "--workload", ACTIVE, "--expires-days", "14"], "Choose an expiry of 7, 30 or 90 days."),
        (&["--requirement", "tasks", "--workload", DISABLED], "Choose an enabled database-access workload in this environment."),
        (&["--requirement", "tasks", "--workload", "104200000000000000002"], "disabled"),
        (&["--requirement", "tasks", "--workload", SECRETS_ONLY], "not registered for database access"),
        (&["--requirement", "tasks", "--workload", MISSING], "Choose an enabled database-access workload in this environment."),
    ];
    for (args, message) in cases {
        let mut full = vec!["grant", "--environment", "production"];
        full.extend_from_slice(args);
        let output = axiom(&cloud, directory.path(), &full);
        assert!(!output.status.success(), "{full:?} succeeded");
        assert!(stderr(&output).contains(message), "{full:?}: {}", stderr(&output));
    }
    let late = (chrono::Utc::now() + chrono::Duration::days(91)).to_rfc3339();
    let output = axiom(&cloud, directory.path(), &["grant", "--resource", RESOURCE, "--requirement", "tasks", "--workload", ACTIVE, "--expires-at", &late]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("--expires-at"), "{}", stderr(&output));
    let output = axiom(&cloud, directory.path(), &["grant", "--resource", RESOURCE, "--requirement", "tasks", "--workload", ACTIVE, "--operation-id", "not-a-uuid"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("--operation-id must be a UUID"));
    assert!(posts(&log).is_empty(), "{:?}", posts(&log));
}

#[test]
fn operation_id_and_expires_at_repeat_the_same_request() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let operation = "6f0e1d2c-3b4a-4596-8877-665544332211";
    let expires = (chrono::Utc::now() + chrono::Duration::days(10)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let args = ["grant", "--resource", RESOURCE, "--requirement", "tasks", "--workload", ACTIVE, "--operation-id", operation, "--expires-at", &expires, "--json"];
    let first = axiom(&cloud, directory.path(), &args);
    assert!(first.status.success(), "{}", stderr(&first));
    let second = axiom(&cloud, directory.path(), &args);
    assert!(second.status.success(), "{}", stderr(&second));
    let value: Value = serde_json::from_str(&stdout(&first)).unwrap();
    assert_eq!(value["grant"]["requirement"], "tasks");
    let posts = posts(&log);
    assert_eq!(posts.len(), 2);
    assert_eq!(posts[0].headers["idempotency-key"], operation);
    assert_eq!(posts[1].headers["idempotency-key"], operation);
    assert_eq!(posts[0].body, posts[1].body);
    let body: Value = serde_json::from_str(&posts[0].body).unwrap();
    assert_eq!(body["expires_at"], expires.replace('Z', ".000Z"));

    // An unconfirmed grant prints how to retry it without a second grant.
    let flaky = axiom(&cloud, directory.path(), &["grant", "--resource", RESOURCE, "--requirement", "flaky", "--workload", ACTIVE, "--operation-id", operation]);
    assert!(!flaky.status.success());
    let text = stderr(&flaky);
    assert!(text.contains("Database access is unavailable right now. Try again shortly."), "{text}");
    let sent: Value = serde_json::from_str(&self::posts(&log)[2].body).unwrap();
    assert!(text.contains(&format!("--operation-id {operation} --expires-at {}", sent["expires_at"].as_str().unwrap())), "{text}");
}

#[test]
fn revoke_posts_an_empty_body_and_prints_the_revocation_bound() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["revoke", GRANT, "--environment", "production"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let posts = posts(&log);
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].path, format!("/api/v1/projects/shop/resources/{RESOURCE}/database-grants/{GRANT}/revoke"));
    assert_eq!(posts[0].body, "{}");
    uuid::Uuid::parse_str(&posts[0].headers["idempotency-key"]).unwrap();
    assert_eq!(
        stdout(&output).trim(),
        "Grant for tasks revoked in production. The workload can't get new credentials. A credential it already holds keeps working until it expires, at most 15 minutes."
    );
    let operation = "7a6b5c4d-3e2f-4a1b-9c8d-7e6f5a4b3c2d";
    let output = axiom(&cloud, directory.path(), &["revoke", GRANT, "--resource", RESOURCE, "--operation-id", operation, "--json"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(self::posts(&log)[1].headers["idempotency-key"], operation);
    let value: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(value["grant"]["id"], GRANT);
}

#[test]
fn conflicts_map_to_the_dashboard_messages() {
    let (cloud, _) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["grant", "--environment", "production", "--requirement", "taken", "--workload", ACTIVE]);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("AXRESOURCE_CONFLICT: This workload already has an active grant for this requirement, or the request changed. Refresh and review the grants."), "{text}");
    assert!(!text.contains("conflicts with the current state"), "server text is never echoed: {text}");
    let output = axiom(&cloud, directory.path(), &["revoke", MISSING, "--environment", "production"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("AXRESOURCE_CONFLICT: The grant changed. Refresh and review it."), "{}", stderr(&output));
    let output = axiom(&cloud, directory.path(), &["grants", "--environment", "staging"]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("No database matches"));
}
