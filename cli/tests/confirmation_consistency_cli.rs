//! Confirmation refusals must precede credentials, target lookup, and HTTP traffic.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
};

struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (count, done) = (requests.clone(), stop.clone());
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
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                count.fetch_add(1, Ordering::SeqCst);
                let mut bytes = Vec::new();
                let mut byte = [0];
                while !bytes.ends_with(b"\r\n\r\n") && stream.read_exact(&mut byte).is_ok() {
                    bytes.push(byte[0]);
                }
                let headers = String::from_utf8_lossy(&bytes);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                let _ = stream.read_exact(&mut body);
                let path = headers
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("");
                let (status, content_type, response) = match path {
                    "/api/v1/projects" => (200, "application/json", r#"[{"id":"project","slug":"demo","name":"Demo"}]"#),
                    "/api/v1/projects/demo/configuration-environments" => (200, "application/json", r#"[{"id":"11111111-1111-4111-8111-111111111111","slug":"development","name":"development"}]"#),
                    "/api/v1/projects/demo/data/environments/11111111-1111-4111-8111-111111111111/tables/items/export" => (200, "text/csv", "id\n1\n# complete 1 rows\n"),
                    _ => (500, "application/json", r#"{"error":"unexpected request"}"#),
                };
                let _ = write!(stream, "HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len());
            }
        });
        Self {
            url,
            requests,
            stop,
            worker: Some(worker),
        }
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
fn sign_in(dir: &std::path::Path) {
    std::fs::write(
        dir.join("auth.json"),
        r#"{"access_token":"stub-access","refresh_token":"stub-refresh","projects":{}}"#,
    )
    .unwrap();
}

#[test]
fn all_destructive_groups_refuse_without_requests_or_credentials() {
    let server = Server::start();
    let id = "11111111-1111-4111-8111-111111111111";
    let cases = [
        vec!["data", "sql", "write", "--file", "missing.json"],
        vec!["data", "tables", "items", "write", "--file", "missing.json"],
        vec!["data", "saved-queries", "delete", id],
        vec!["data", "capture", "create", "--file", "missing.json"],
        vec![
            "data",
            "capture",
            "settings",
            "set",
            "--file",
            "missing.json",
        ],
        vec!["workspace", "members", "remove", "missing@example.test"],
        vec!["workspace", "invitations", "cancel", id],
        vec![
            "project",
            "environments",
            "reset",
            "--environment",
            "missing",
        ],
        vec![
            "project",
            "environments",
            "delete",
            "--environment",
            "missing",
        ],
        vec![
            "intel",
            "alert-policies",
            "update",
            "errors",
            "--enabled",
            "false",
        ],
        vec![
            "intel",
            "observability",
            "settings",
            "update",
            "--body",
            "{}",
        ],
        vec!["intel", "observability", "sources", "revoke", id],
        vec!["intel", "pull-requests", "update", "1", "--body", "{}"],
        vec!["intel", "settings", "update", "--name", "Renamed"],
    ];
    for signed_in in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        if signed_in {
            sign_in(dir.path());
        }
        for args in &cases {
            let output = cli(&server, dir.path(), args);
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{args:?}, signed_in={signed_in}: {stderr}"
            );
            assert_eq!(
                stderr.trim(),
                format!("axiom {}: pass --yes to confirm without a prompt", args[0]),
                "{args:?}"
            );
            assert!(output.stdout.is_empty(), "{args:?}");
            assert_eq!(server.requests.load(Ordering::SeqCst), 0, "{args:?}");
        }
    }
}

#[test]
fn export_dash_writes_only_csv_to_stdout_and_preserves_dash_file() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    sign_in(dir.path());
    std::fs::write(dir.path().join("input.json"), "{}").unwrap();
    for force in [false, true] {
        if force {
            std::fs::write(dir.path().join("-"), "keep me").unwrap();
        }
        for json in [false, true] {
            let mut args = vec![
                "data",
                "--project",
                "demo",
                "--environment",
                "development",
                "tables",
                "items",
                "export",
                "--file",
                "input.json",
                "--out",
                "-",
            ];
            if force {
                args.push("--force");
            }
            if json {
                args.push("--json");
            }
            let output = cli(&server, dir.path(), &args);
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, b"id\n1\n# complete 1 rows\n");
            if force {
                assert_eq!(
                    std::fs::read_to_string(dir.path().join("-")).unwrap(),
                    "keep me"
                );
            } else {
                assert!(!dir.path().join("-").exists());
            }
        }
    }
    assert_eq!(server.requests.load(Ordering::SeqCst), 12);
}

#[cfg(unix)]
#[test]
fn intel_prompts_on_a_terminal_without_yes() {
    let server = Server::start();
    let dir = tempfile::tempdir().unwrap();
    sign_in(dir.path());
    // Python's standard-library PTY keeps this integration check independent of
    // a terminal emulator and lets us answer the real dialoguer confirmation.
    let script = r#"
import errno, os, pty, select, subprocess, sys, time
for answer, expected in [(b'n\n', b'cancelled'), (b'y\n', b'Pass --project')]:
    master, slave = pty.openpty()
    child = subprocess.Popen([sys.argv[1], 'intel', 'settings', 'update', '--name', 'Renamed'], stdin=slave, stdout=subprocess.PIPE, stderr=slave)
    os.close(slave)
    output = bytearray()
    answered = False
    deadline = time.monotonic() + 10
    try:
        while time.monotonic() < deadline:
            if select.select([master], [], [], 0.05)[0]:
                try:
                    chunk = os.read(master, 65536)
                except OSError as e:
                    if e.errno == errno.EIO: break
                    raise
                if not chunk: break
                output.extend(chunk)
                if not answered and b'Apply this change' in output:
                    os.write(master, answer)
                    answered = True
        if child.poll() is None: child.kill()
        stdout, _ = child.communicate(timeout=5)
        assert answered, output
        assert expected in output, output
        assert b'pass --yes' not in output, output
        assert not stdout, stdout
    finally:
        if child.poll() is None: child.kill()
        child.wait()
        os.close(master)
"#;
    let output = Command::new("python3")
        .args(["-c", script, env!("CARGO_BIN_EXE_axiom-cli")])
        .current_dir(dir.path())
        .env("AXIOM_CLOUD_URL", &server.url)
        .env("AXIOM_LOCAL_PROFILE_DIR", dir.path())
        .env("HOME", dir.path())
        .env("CI", "true")
        .env_remove("AXIOM_REFERRAL_CODE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}
