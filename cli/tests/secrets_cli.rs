//! `axiom secrets` Cloud commands against a loopback stub of the secrets API:
//! names for environments and secrets, revisions in `versions`, an optional
//! `--revision`, and `rotate` (W4-L5).
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

const ENV: &str = "3a4b5c6d-7e8f-4a0b-8c1d-2e3f4a5b6c7d";
const SECRET: &str = "5e6f7a8b-9c0d-4e1f-8a2b-3c4d5e6f7a8b";
const V1: &str = "6f7a8b9c-0d1e-4f2a-8b3c-4d5e6f7a8b9c";
const V2: &str = "7a8b9c0d-1e2f-4a3b-8c4d-5e6f7a8b9c0d";
const V3: &str = "8b9c0d1e-2f3a-4b4c-8d5e-6f7a8b9c0d1e";
const VALUE: &str = "SYNTHETIC_ROTATED_VALUE_0123";

#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

type Log = Arc<Mutex<Vec<Request>>>;
type Route = fn(&Request, &[Request]) -> (u16, String);

/// Serve canned responses on 127.0.0.1. `route` sees the request and every
/// earlier request, so a stub can change after a write. Every request is
/// recorded with lower-cased header names.
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
            let mut headers = Vec::new();
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" || header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    let name = name.trim().to_ascii_lowercase();
                    let value = value.trim().to_string();
                    if name == "content-length" {
                        length = value.parse().unwrap_or(0);
                    }
                    headers.push((name, value));
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let request = Request { method, path, headers, body: String::from_utf8_lossy(&body).to_string() };
            let earlier = recorded.lock().unwrap().clone();
            let (status, response) = route(&request, &earlier);
            recorded.lock().unwrap().push(request);
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
        // A command that refuses exits without reading stdin; a broken pipe is fine then.
        let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    }
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}

fn base() -> String {
    "/api/v1/projects/shop".into()
}

fn scope() -> String {
    format!("{}/configuration-environments/{ENV}", base())
}

fn route(request: &Request, earlier: &[Request]) -> (u16, String) {
    let rotated = earlier.iter().any(|r| r.method == "POST" && r.path == format!("{}/secrets/{SECRET}/versions", scope()));
    let versions = if rotated {
        format!(
            r#"[{{"id":"{V3}","version":3,"status":"candidate","created_at":"2026-11-13T10:00:00Z","destroy_after":null}},{{"id":"{V2}","version":2,"status":"candidate","created_at":"2026-11-12T10:00:00Z","destroy_after":null}},{{"id":"{V1}","version":1,"status":"active","created_at":"2026-11-01T10:00:00Z","destroy_after":null}}]"#
        )
    } else {
        format!(
            r#"[{{"id":"{V2}","version":2,"status":"candidate","created_at":"2026-11-12T10:00:00Z","destroy_after":null}},{{"id":"{V1}","version":1,"status":"disabled","created_at":"2026-11-01T10:00:00Z","destroy_after":"2026-11-20T10:00:00Z"}}]"#
        )
    };
    let path = request.path.as_str();
    match request.method.as_str() {
        "GET" if path == format!("{}/configuration-environments", base()) => (
            200,
            format!(r#"[{{"id":"0b5e2c1a-9f3d-4c7e-8a21-5d6f7e8a9b0c","slug":"development","kind":"development","protected":false,"status":"active","revision":1,"expires_at":null}},{{"id":"{ENV}","slug":"production","kind":"production","protected":true,"status":"active","revision":2,"expires_at":null}}]"#),
        ),
        "GET" if path == format!("{}/secrets", base()) => (
            200,
            format!(r#"[{{"id":"9c0d1e2f-3a4b-4c5d-8e6f-7a8b9c0d1e2f","name":"OTHER_KEY","purpose":"authenticateHttp"}},{{"id":"{SECRET}","name":"STRIPE_KEY","purpose":"authenticateHttp"}}]"#),
        ),
        "GET" if path == format!("{}/secrets", scope()) => (
            200,
            format!(r#"[{{"id":"{SECRET}","name":"STRIPE_KEY","purpose":"authenticateHttp","active_version_id":null,"revision":7,"active_version":null,"latest_version":2,"latest_version_status":"candidate","last_value_created_at":"2026-11-12T10:00:00Z","version_count":2,"candidate_count":1}}]"#),
        ),
        "GET" if path == format!("{}/secrets/{SECRET}/versions", scope()) => (200, versions),
        "POST" if path == format!("{}/secrets/{SECRET}/versions", scope()) => (200, format!(r#"{{"id":"{V3}","status":"candidate"}}"#)),
        "POST" if path.starts_with(&format!("{}/secrets/{SECRET}/versions/", scope())) => (200, r#"{"updated":true}"#.into()),
        _ => (404, r#"{"error":"not found"}"#.into()),
    }
}

fn conflicting(request: &Request, earlier: &[Request]) -> (u16, String) {
    if request.method == "POST" && request.path.ends_with("/activate") {
        return (409, r#"{"error":"conflict"}"#.into());
    }
    route(request, earlier)
}

fn calls(log: &Log) -> Vec<(String, String)> {
    log.lock().unwrap().iter().map(|r| (r.method.clone(), r.path.clone())).collect()
}

#[test]
fn versions_prints_the_revision_and_one_line_per_version() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["secrets", "versions", "STRIPE_KEY", "--project", "shop", "--environment", "production"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("STRIPE_KEY") && stdout.contains("production"), "{stdout}");
    assert!(stdout.lines().any(|l| l.contains("Revision") && l.contains('7')), "{stdout}");
    let v2 = stdout.lines().find(|l| l.trim_start().starts_with("v2 ")).unwrap_or_else(|| panic!("{stdout}"));
    assert!(v2.contains(V2) && v2.contains("candidate") && v2.contains("2026-11-12T10:00:00Z"), "{v2}");
    let v1 = stdout.lines().find(|l| l.trim_start().starts_with("v1 ")).unwrap_or_else(|| panic!("{stdout}"));
    assert!(v1.contains(V1) && v1.contains("disabled") && v1.contains("2026-11-20T10:00:00Z"), "{v1}");
    assert!(
        stdout.contains("axiom secrets activate STRIPE_KEY v2 --project shop --environment production --revision 7"),
        "{stdout}"
    );
    let requests = calls(&log);
    assert!(requests.contains(&("GET".into(), format!("{}/secrets/{SECRET}/versions", scope()))), "{requests:?}");
    assert!(requests.contains(&("GET".into(), format!("{}/secrets", scope()))), "{requests:?}");

    let output = axiom(&cloud, directory.path(), &["secrets", "versions", SECRET, "--project", "shop", "--environment", ENV, "--json"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["secret"], "STRIPE_KEY");
    assert_eq!(json["secret_id"], SECRET);
    assert_eq!(json["environment_id"], ENV);
    assert_eq!(json["revision"], 7);
    assert_eq!(json["active_version_id"], serde_json::Value::Null);
    assert_eq!(json["versions"].as_array().unwrap().len(), 2);
    assert_eq!(json["versions"][0]["version"], 2);
}

#[test]
fn names_resolve_to_uuids_in_the_request_path() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["secrets", "workloads", "--project", "shop", "--environment", "production"], None);
    // The stub has no workloads route; only the path matters here.
    let _ = output;
    let requests = calls(&log);
    assert_eq!(requests[0], ("GET".into(), format!("{}/configuration-environments", base())));
    assert_eq!(requests[1], ("GET".into(), format!("{}/secret-workloads", scope())));
    // A UUID passes through without a lookup.
    log.lock().unwrap().clear();
    let _ = axiom(&cloud, directory.path(), &["secrets", "grants", "--project", "shop", "--environment", ENV], None);
    assert_eq!(calls(&log), vec![("GET".into(), format!("{}/secret-grants", scope()))]);
}

#[test]
fn activate_without_revision_sends_the_listed_revision() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["secrets", "activate", "STRIPE_KEY", "v2", "--project", "shop", "--environment", "production", "--yes"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let requests = log.lock().unwrap().clone();
    let activate = requests.last().unwrap();
    assert_eq!(activate.method, "POST");
    assert_eq!(activate.path, format!("{}/secrets/{SECRET}/versions/{V2}/activate", scope()));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&activate.body).unwrap(), serde_json::json!({"expected_revision":7}));

    // With UUIDs and an explicit revision, nothing is looked up.
    log.lock().unwrap().clear();
    let output = axiom(&cloud, directory.path(), &["secrets", "revoke", SECRET, V1, "--project", "shop", "--environment", ENV, "--revision", "3", "--yes"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let requests = log.lock().unwrap().clone();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].path, format!("{}/secrets/{SECRET}/versions/{V1}/disable", scope()));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&requests[0].body).unwrap(), serde_json::json!({"expected_revision":3}));
}

#[test]
fn upper_case_uuids_match_the_listed_ids() {
    // The backend lists IDs in lower case; an upper-case secret UUID must still find its
    // revision (7), not fall back to 1.
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let upper = SECRET.to_ascii_uppercase();
    let output = axiom(&cloud, directory.path(), &["secrets", "activate", &upper, &V2.to_ascii_uppercase(), "--project", "shop", "--environment", &ENV.to_ascii_uppercase(), "--yes"], None);
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let requests = log.lock().unwrap().clone();
    let activate = requests.last().unwrap();
    assert_eq!(activate.path, format!("{}/secrets/{SECRET}/versions/{V2}/activate", scope()));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&activate.body).unwrap(), serde_json::json!({"expected_revision":7}));
}

#[test]
fn rotate_writes_then_activates_with_the_current_revision() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["secrets", "rotate", "STRIPE_KEY", "--project", "shop", "--environment", "production", "--stdin", "--yes"], Some(VALUE));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stdout.contains(VALUE) && !stderr.contains(VALUE));
    assert!(stdout.contains(&format!("Saved v3 ({V3}) and activated it in production (revision now 8).")), "{stdout}");
    let requests = log.lock().unwrap().clone();
    let write = requests.iter().position(|r| r.method == "POST" && r.path == format!("{}/secrets/{SECRET}/versions", scope())).expect("write");
    let activate = requests.iter().position(|r| r.method == "POST" && r.path.ends_with("/activate")).expect("activate");
    assert!(write < activate);
    assert_eq!(requests[write].body, VALUE);
    assert_eq!(requests[write].header("content-type"), Some("application/octet-stream"));
    assert!(requests[write].header("idempotency-key").is_some_and(|k| k.len() == 36));
    assert_eq!(requests[activate].path, format!("{}/secrets/{SECRET}/versions/{V3}/activate", scope()));
    assert_eq!(serde_json::from_str::<serde_json::Value>(&requests[activate].body).unwrap(), serde_json::json!({"expected_revision":7}));
    // No request other than the write carries the value.
    assert_eq!(requests.iter().filter(|r| r.body.contains(VALUE)).count(), 1);
}

#[test]
fn rotate_whose_activation_fails_prints_the_follow_up_command() {
    let (cloud, log) = stub(conflicting);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["secrets", "rotate", "STRIPE_KEY", "--project", "shop", "--environment", "production", "--stdin", "--yes"], Some(VALUE));
    assert!(!output.status.success());
    let all = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    assert!(!all.contains(VALUE));
    assert!(all.contains("candidate was saved"), "{all}");
    // rotate ran with --yes, so the follow-up runs headless too.
    assert!(all.contains("axiom secrets activate STRIPE_KEY v3 --project shop --environment production --yes"), "{all}");
    assert!(log.lock().unwrap().iter().any(|r| r.path.ends_with("/activate")));
}

#[test]
fn unknown_environment_name_errors_without_secret_requests() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let output = axiom(&cloud, directory.path(), &["secrets", "versions", "STRIPE_KEY", "--project", "shop", "--environment", "staging"], None);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(r#"unknown environment "staging"; pass a configured environment name or UUID"#), "{stderr}");
    assert_eq!(calls(&log), vec![("GET".into(), format!("{}/configuration-environments", base()))]);

    let output = axiom(&cloud, directory.path(), &["secrets", "versions", "NOPE", "--project", "shop", "--environment", ENV], None);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(r#"unknown secret "NOPE""#));
}

/// Every version action, grant revocation, workload disable and rotate asks first, as the
/// dashboard does. Without a terminal (stdin is a pipe here) and without --yes, the command
/// refuses before any request; with --yes it sends.
#[test]
fn destructive_commands_without_a_terminal_need_yes_and_send_nothing() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    const GRANT: &str = "1d2e3f4a-5b6c-4d7e-8f9a-0b1c2d3e4f5a";
    let scoped = ["--project", "shop", "--environment", ENV];
    let commands: Vec<(Vec<&str>, String)> = vec![
        (vec!["secrets", "revoke", SECRET, V1, "--revision", "3"], format!("{}/secrets/{SECRET}/versions/{V1}/disable", scope())),
        (vec!["secrets", "destroy", SECRET, V1, "--revision", "3"], format!("{}/secrets/{SECRET}/versions/{V1}/destroy", scope())),
        (vec!["secrets", "activate", SECRET, V2, "--revision", "3"], format!("{}/secrets/{SECRET}/versions/{V2}/activate", scope())),
        (vec!["secrets", "rewrap", SECRET, V2, "--revision", "3"], format!("{}/secrets/{SECRET}/versions/{V2}/rewrap", scope())),
        (vec!["secrets", "revoke-grant", GRANT], format!("{}/secret-grants/{GRANT}/revoke", scope())),
        (vec!["secrets", "disable-workload", GRANT], format!("{}/secret-workloads/{GRANT}/disable", scope())),
    ];
    for (command, path) in &commands {
        let args: Vec<&str> = command.iter().chain(scoped.iter()).copied().collect();
        log.lock().unwrap().clear();
        let output = axiom(&cloud, directory.path(), &args, None);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{command:?} ran without --yes");
        assert!(stderr.contains("pass --yes to confirm without a prompt"), "{command:?}: {stderr}");
        assert!(calls(&log).is_empty(), "{command:?} sent {:?}", calls(&log));

        let args: Vec<&str> = args.into_iter().chain(["--yes"]).collect();
        let output = axiom(&cloud, directory.path(), &args, None);
        // The stub answers version actions; grant and workload routes are not stubbed (404).
        if path.contains("/versions/") {
            assert!(output.status.success(), "{command:?}: {}", String::from_utf8_lossy(&output.stderr));
        }
        let requests = calls(&log);
        assert_eq!(requests.last(), Some(&("POST".to_string(), path.clone())), "{command:?}: {requests:?}");
    }

    // rotate refuses before reading the value or sending anything.
    log.lock().unwrap().clear();
    let output = axiom(&cloud, directory.path(), &["secrets", "rotate", "STRIPE_KEY", "--project", "shop", "--environment", "production", "--stdin"], Some(VALUE));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("pass --yes to confirm without a prompt") && !stderr.contains(VALUE), "{stderr}");
    assert!(calls(&log).is_empty(), "{:?}", calls(&log));
}

/// Enabling or disabling an environment asks first, as the dashboard does; a protection or
/// expiry change alone does not.
#[test]
fn update_environment_status_change_needs_yes_without_a_terminal() {
    let (cloud, log) = stub(route);
    let directory = tempfile::tempdir().unwrap();
    profile(directory.path());
    let write = |name: &str, body: &str| {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.path().join(name);
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path.to_string_lossy().into_owned()
    };
    let disable = write("disable.json", r#"{"status":"disabled","protected":true,"expected_revision":2,"expires_at":null}"#);
    let protect = write("protect.json", r#"{"protected":true,"expected_revision":2}"#);
    let run = |file: &str, yes: bool| {
        let mut args = vec!["secrets", "update-environment", "--project", "shop", "--environment", ENV, "--file", file];
        if yes {
            args.push("--yes");
        }
        axiom(&cloud, directory.path(), &args, None)
    };
    let output = run(&disable, false);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("pass --yes to confirm without a prompt"), "{stderr}");
    assert!(calls(&log).is_empty(), "{:?}", calls(&log));
    let _ = run(&disable, true);
    assert_eq!(calls(&log), vec![("PATCH".to_string(), format!("{}/", scope()))]);
    log.lock().unwrap().clear();
    let _ = run(&protect, false);
    assert_eq!(calls(&log), vec![("PATCH".to_string(), format!("{}/", scope()))]);
}
