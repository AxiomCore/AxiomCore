//! `axiom deploy frontend` and `axiom deployments` through the real binary against a loopback
//! API: the saved session, project and environment lookup, stdout JSON only, the resume
//! journal beside `auth.json`, and exit codes with the next step.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

const ENV: &str = "11111111-2222-3333-4444-555555555555";
const B: &str = "/api/v1/projects/demo/environments/11111111-2222-3333-4444-555555555555/deployments";

struct Server {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
    /// `METHOD path` and the request body, in order.
    bodies: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

fn reply(method: &str, path: &str) -> (u16, String) {
    // The backend slot answers like the frontend one.
    let generic = path.replace(&format!("{B}/backend"), &format!("{B}/frontend"));
    let path = generic.as_str();
    let b = |rest: &str| format!("{B}{rest}");
    let body = match (method, path) {
        ("GET", "/api/v1/projects") => r#"[{"id":"p1","slug":"demo","name":"Demo","visibility":"private","created_at":"t","updated_at":"t"}]"#.to_string(),
        ("GET", "/api/v1/projects/demo/configuration-environments") => format!(r#"[{{"id":"{ENV}","slug":"development"}}]"#),
        ("GET", p) if p == b("") => r#"{"items":[{"kind":"frontend","state":"active","active_version_id":"ver_1","url":"https://demo-development-a1b2c3.axiomcore.page","updated_at":"2026-11-16T10:00:00Z"}],"next_cursor":null}"#.into(),
        ("GET", p) if p == b("/frontend") => return (404, r#"{"code":"AXDEPLOY_NOT_FOUND"}"#.into()),
        ("GET", p) if p == b("/frontend/health") => return (503, r#"{"code":"AXDEPLOY_DISABLED","request_id":"req_7"}"#.into()),
        ("POST", p) if p == b("/frontend/uploads") => r#"{"id":"upl_1","state":"open","missing_hashes":[]}"#.into(),
        ("POST", p) if p == b("/frontend/uploads/upl_1/complete") => r#"{"id":"upl_1","state":"sealed"}"#.into(),
        ("POST", p) if p == b("/frontend/versions") => r#"{"version_id":"ver_1","operation_id":"op_v","url":"https://demo-development-a1b2c3.axiomcore.page"}"#.into(),
        ("GET", p) if p == b("/frontend/operations/op_v") => r#"{"operation_id":"op_v","state":"succeeded"}"#.into(),
        ("GET", p) if p == b("/frontend/versions/ver_1") => r#"{"id":"ver_1","status":"ready"}"#.into(),
        _ => return (500, r#"{"error":"unexpected request"}"#.into()),
    };
    (if method == "POST" && path.ends_with("/versions") { 202 } else { 200 }, body)
}

/// Someone rolled back to ver_0 (generation 2) while this deploy ran: the activation
/// finishes, but the slot still serves ver_0.
fn reply_superseded(method: &str, path: &str) -> (u16, String) {
    let b = |rest: &str| format!("{B}{rest}");
    match (method, path) {
        ("GET", p) if p == b("/frontend") => (200, r#"{"deployment_id":"dpl_1","kind":"frontend","generation":2,"state":"active","active_version_id":"ver_0","url":"https://demo-development-a1b2c3.axiomcore.page"}"#.into()),
        ("POST", p) if p == b("/frontend/versions/ver_1/activate") => (202, r#"{"deployment_id":"dpl_1","generation":2,"state":"active","active_version_id":"ver_0","operation_id":"op_a"}"#.into()),
        ("GET", p) if p == b("/frontend/operations/op_a") => (200, r#"{"operation_id":"op_a","state":"succeeded"}"#.into()),
        _ => reply(method, path),
    }
}

impl Server {
    fn start() -> Self {
        Self::start_with(reply)
    }
    fn start_with(reply: fn(&str, &str) -> (u16, String)) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, body_log, done) = (seen.clone(), bodies.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(2));
                        continue;
                    }
                    Err(e) => panic!("{e}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
                let mut bytes = Vec::new();
                let mut byte = [0];
                while !bytes.ends_with(b"\r\n\r\n") && stream.read_exact(&mut byte).is_ok() {
                    bytes.push(byte[0]);
                }
                let headers = String::from_utf8_lossy(&bytes).to_string();
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                let _ = stream.read_exact(&mut body);
                let mut first = headers.lines().next().unwrap_or("").split_whitespace();
                let method = first.next().unwrap_or("").to_string();
                let target = first.next().unwrap_or("").to_string();
                let path = target.split('?').next().unwrap_or("").to_string();
                log.lock().unwrap().push(format!("{method} {path}"));
                body_log.lock().unwrap().push((format!("{method} {path}"), String::from_utf8_lossy(&body).to_string()));
                let (status, response) = reply(&method, &path);
                let _ = write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len());
            }
        });
        Self { url, seen, bodies, stop, worker: Some(worker) }
    }
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
    /// The JSON bodies sent to `METHOD path`.
    fn bodies(&self, route: &str) -> Vec<serde_json::Value> {
        self.bodies.lock().unwrap().iter().filter(|(r, _)| r == route).map(|(_, b)| serde_json::from_str(b).unwrap()).collect()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn cli(server: &Server, dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .args(args)
        .current_dir(dir)
        .env("AXIOM_CLOUD_URL", &server.url)
        .env("AXIOM_LOCAL_PROFILE_DIR", dir)
        .env("HOME", dir)
        .env("CI", "true")
        .env_remove("AXIOM_REFERRAL_CODE")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn signed_in() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("auth.json"), r#"{"access_token":"stub-access","refresh_token":"stub-refresh","projects":{}}"#).unwrap();
    std::fs::create_dir_all(dir.path().join("dist/assets")).unwrap();
    std::fs::write(dir.path().join("dist/index.html"), "<h1>hi</h1>").unwrap();
    std::fs::write(dir.path().join("dist/assets/app.js"), "console.log(1)").unwrap();
    dir
}

#[test]
fn deploy_frontend_no_activate_prints_one_json_object_and_journals_the_intent() {
    let server = Server::start();
    let dir = signed_in();
    let out = cli(&server, dir.path(), &["deploy", "frontend", "--project", "demo", "--no-activate", "--wait", "--json"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is one JSON object");
    assert_eq!(
        value,
        serde_json::json!({"schema": "axiom-deployments/v1", "kind": "frontend", "project": "demo", "environment": "development", "version_id": "ver_1", "operation_id": "op_v", "url": "https://demo-development-a1b2c3.axiomcore.page", "state": "ready", "activated": false})
    );
    let seen = server.seen();
    assert_eq!(seen[..2], ["GET /api/v1/projects", "GET /api/v1/projects/demo/configuration-environments"]);
    assert!(seen.contains(&format!("POST {B}/frontend/versions")) && !seen.iter().any(|r| r.ends_with("/activate")), "{seen:?}");
    let journal: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join(format!("deployments/demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_eq!((journal["version_id"].as_str(), journal["version_ready"].as_bool()), (Some("ver_1"), Some(true)));
    assert!(!stderr.contains("stub-access"));

    // Human output names the next command.
    let out = cli(&server, dir.path(), &["deploy", "frontend", "dist", "--project", "demo", "--no-activate"]);
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "Version ver_1 is ready. Activate it with: axiom deployments activate frontend ver_1 --project demo");
}

#[test]
fn deployments_list_and_typed_errors_through_the_binary() {
    let server = Server::start();
    let dir = signed_in();
    let out = cli(&server, dir.path(), &["deployments", "list", "--project", "demo"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.starts_with("KIND      STATE   ACTIVE VERSION  URL") && text.contains("frontend  active  ver_1"), "{text}");
    let out = cli(&server, dir.path(), &["deployments", "health", "backend", "--project", "demo"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr).trim(),
        "axiom deployments: AXDEPLOY_DISABLED: This project is not in the Axiom hosting pilot yet (deployments are invite-only). Ask the Axiom team to add it. (request req_7)"
    );
    let out = cli(&server, dir.path(), &["deployments", "logs", "frontend", "--project", "demo", "--since", "yesterday"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--since takes a duration such as 30s, 10m, 1h or 2d"));
    assert_eq!(server.seen().len(), 6, "a bad --since is refused before any request");
}

const SOURCE: &str = r#"project(id: "docs.hello", version: "1.0.0")
variant default(include: ["*"])
model Greeting { message: String }
action hello(kind: ActionKind.query, output: Models.Greeting) {
  managed { return {message: "Hello from Acore"} }
}
endpoint hello(method: GET, path: "/hello") {
  handle(action: Actions.hello)
  response(type: Models.Greeting)
}
"#;

/// Without --from-input, `deploy backend` packages backend.axiom + backend.server.json +
/// deployment.json in-process (as `axiom server package --output-input`) and uploads it.
#[test]
fn deploy_backend_packages_the_current_project() {
    let server = Server::start();
    let dir = signed_in();
    let d = dir.path();
    std::fs::write(d.join("main.acore"), SOURCE).unwrap();
    std::fs::write(d.join("AxiomDeps.toml"), "type = \"backend\"\ntargets = [\"server\"]\naudiences = [\"web\"]\n").unwrap();
    for args in [vec!["build", "main.acore", "--out", "backend.axiom"], vec!["server", "deployment", "backend.axiom", "backend.server.json", "--output", "deployment.json"]] {
        let out = cli(&server, d, &args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    let before = server.seen().len();
    let out = cli(&server, d, &["deploy", "backend", "--project", "demo", "--no-activate", "--json"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!((value["kind"].as_str(), value["version_id"].as_str(), value["activated"].as_bool()), (Some("backend"), Some("ver_1"), Some(false)));
    let seen = server.seen()[before..].to_vec();
    assert!(seen.contains(&format!("POST {B}/backend/uploads")) && seen.contains(&format!("POST {B}/backend/versions")), "{seen:?}");
    let journal: serde_json::Value = serde_json::from_slice(&std::fs::read(d.join(format!("deployments/demo-{ENV}-backend.json"))).unwrap()).unwrap();
    assert_eq!(journal["kind"], "backend");
}

/// A4.3: there is no source-map option; a source map in the build stops the deploy locally
/// (exit 1, nothing sent), and the old flag is an unknown argument (exit 2, nothing sent).
#[test]
fn source_maps_are_refused_and_the_include_flag_is_gone() {
    let server = Server::start();
    let dir = signed_in();
    std::fs::write(dir.path().join("dist/assets/app.js.map"), "{}").unwrap();
    let out = cli(&server, dir.path(), &["deploy", "frontend", "--project", "demo", "--no-activate"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("/assets/app.js.map: source maps are not published"), "{stderr}");
    let out = cli(&server, dir.path(), &["deploy", "frontend", "--include-source-maps", "--project", "demo", "--no-activate"]);
    assert_eq!(out.status.code(), Some(2), "{}", String::from_utf8_lossy(&out.stderr));
    let help = cli(&server, dir.path(), &["deploy", "frontend", "--help"]);
    assert!(!String::from_utf8_lossy(&help.stdout).contains("source-maps"));
    assert!(server.seen().is_empty(), "{:?}", server.seen());
}

/// S2: signed out, or the Cloud unreachable, each says so in one sentence (no OS error tail).
#[test]
fn signed_out_and_unreachable_are_told_apart() {
    let server = Server::start();
    let dir = signed_in();
    std::fs::remove_file(dir.path().join("auth.json")).unwrap();
    let out = cli(&server, dir.path(), &["deployments", "list", "--project", "demo"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("Not signed in") && stderr.contains("`axiom login`") && !stderr.contains("os error"), "{stderr}");

    let dir = signed_in();
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let out = Command::new(env!("CARGO_BIN_EXE_axiom-cli"))
        .args(["deploy", "frontend", "--project", "demo", "--no-activate"])
        .current_dir(dir.path())
        .env("AXIOM_CLOUD_URL", &url)
        .env("AXIOM_LOCAL_PROFILE_DIR", dir.path())
        .env("HOME", dir.path())
        .env("CI", "true")
        .env_remove("AXIOM_REFERRAL_CODE")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("Could not reach Axiom Cloud (") && !stderr.contains("os error") && !stderr.contains("axiom login"), "{stderr}");
}

#[test]
fn a_superseded_activation_is_not_reported_live_and_a_rerun_does_not_override_it() {
    let server = Server::start_with(reply_superseded);
    let dir = signed_in();
    let run = || cli(&server, dir.path(), &["deploy", "frontend", "--project", "demo", "--yes"]);
    let posts = |suffix: &str| server.seen().iter().filter(|r| r.starts_with("POST") && r.ends_with(suffix)).count();
    let out = run();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{stderr}");
    assert!(stderr.contains("Version ver_1 did not become live: the frontend deployment now serves ver_0"), "{stderr}");
    assert!(stderr.contains("axiom deployments activate frontend ver_1 --project demo"), "{stderr}");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("live"), "{}", String::from_utf8_lossy(&out.stdout));
    let journal: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join(format!("deployments/demo-{ENV}-frontend.json"))).unwrap()).unwrap();
    assert_ne!(journal["failed"].as_bool(), Some(true), "{journal}");
    assert_ne!(journal["activated"].as_bool(), Some(true), "{journal}");
    assert_eq!((posts("/versions"), posts("/activate")), (1, 1));

    // A plain rerun resumes the same intent: no new version, the same activation, and it
    // stops again rather than undoing the rollback.
    let out = run();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success() && stderr.contains("did not become live"), "{stderr}");
    assert_eq!((posts("/versions"), posts("/activate")), (1, 2));
}

/// A package input folder (as `axiom server package --output-input` writes it) whose
/// deployment.json binds configuration to `env:GREETING`.
fn greeting_input(dir: &std::path::Path) -> std::path::PathBuf {
    use sha2::Digest;
    let input = dir.join("input");
    std::fs::create_dir_all(&input).unwrap();
    let deployment = r#"{"bindings":{"greeting":{"configuration":{"greeting":"env:GREETING"},"provider":"publicConfiguration"}}}"#;
    let mut listed = serde_json::Map::new();
    for (name, body) in [("contract.axiom", "c"), ("contract.server.json", "{}"), ("deployment.json", deployment)] {
        std::fs::write(input.join(name), body).unwrap();
        let sha: String = sha2::Sha256::digest(body.as_bytes()).iter().map(|b| format!("{b:02x}")).collect();
        listed.insert(name.into(), serde_json::json!({"bytes": body.len(), "executable": false, "sha256": sha}));
    }
    let manifest = serde_json::json!({"artifactReferences": {}, "files": listed, "format": "axiom-container-input/v1", "target": {"id": "container-amd64"}, "targetSha256": "a".repeat(64)});
    std::fs::write(input.join("input.json"), manifest.to_string()).unwrap();
    input
}

/// F1: a backend whose deployment.json binds `env:GREETING` is refused locally without a
/// value (exit 2, no request at all), and deploys with `--env`/`--env-file`, sending `env` in
/// create-version; only the names reach the output, only a hash reaches the journal.
#[test]
fn deploy_backend_env_is_prechecked_and_sent_with_the_version() {
    let server = Server::start();
    let dir = signed_in();
    let d = dir.path();
    let input = greeting_input(d);
    let input = input.to_str().unwrap();

    let out = cli(&server, d, &["deploy", "backend", "--from-input", input, "--project", "demo", "--no-activate"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert_eq!(
        stderr.trim(),
        "axiom deploy: Missing configuration for this backend: GREETING (bound by greeting). Pass it with --env GREETING=... or --env-file .env, then deploy again."
    );
    let out = cli(&server, d, &["deploy", "backend", "--from-input", input, "--project", "demo", "--no-activate", "--env", "GREETING=hi", "--env", "API_KEY=x"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("Invalid configuration name: API_KEY looks like a secret") && stderr.contains("`axiom secrets`"), "{stderr}");
    assert!(server.seen().is_empty(), "nothing is sent before the precheck passes: {:?}", server.seen());

    // A dotenv file plus a flag that overrides it; EXTRA is bound by nothing (noted, sent).
    std::fs::write(d.join(".env"), "# hosted configuration\nGREETING='from file'\nEXTRA=1\n").unwrap();
    let out = cli(&server, d, &["deploy", "backend", "--from-input", input, "--project", "demo", "--no-activate", "--json", "--env-file", ".env", "--env", "GREETING=Hello from the CLI"]);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("Note: EXTRA is not bound by any configuration in deployment.json; sent anyway."), "{stderr}");
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["env_keys"], serde_json::json!(["EXTRA", "GREETING"]));
    let versions = server.bodies(&format!("POST {B}/backend/versions"));
    assert_eq!(versions.len(), 1, "{:?}", server.seen());
    assert_eq!(versions[0]["env"], serde_json::json!({"GREETING": "Hello from the CLI", "EXTRA": "1"}));
    assert_eq!(versions[0]["artifact"]["type"], "container_input");
    let journal = std::fs::read_to_string(d.join(format!("deployments/demo-{ENV}-backend.json"))).unwrap();
    assert!(!journal.contains("Hello from the CLI") && !journal.contains("EXTRA"), "{journal}");
    assert_eq!(serde_json::from_str::<serde_json::Value>(&journal).unwrap()["env_sha256"].as_str().map(str::len), Some(64));

    // Human output names the keys, never the values. Other values are a fresh intent.
    let out = cli(&server, d, &["deploy", "backend", "--from-input", input, "--project", "demo", "--no-activate", "--env", "GREETING=changed"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(stdout.contains("  Configuration: GREETING") && !stdout.contains("changed"), "{stdout}");
    let versions = server.bodies(&format!("POST {B}/backend/versions"));
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[1]["env"], serde_json::json!({"GREETING": "changed"}));
}
