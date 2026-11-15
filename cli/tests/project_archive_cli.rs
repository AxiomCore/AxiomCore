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

/// `axiom project archive` on a pseudo-terminal: the typed prompt is shown before any request.
#[cfg(unix)]
mod terminal {
    use super::*;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::process::CommandExt;
    use std::time::{Duration, Instant};

    const PROMPT: &str = "to confirm";

    struct Session {
        child: std::process::Child,
        master: std::fs::File,
        seen: String,
    }

    fn spawn(cloud: &str, directory: &Path, args: &[&str]) -> Session {
        let (mut master, mut slave) = (-1, -1);
        let opened = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) };
        assert_eq!(opened, 0, "openpty: {}", std::io::Error::last_os_error());
        let slave = unsafe { std::fs::File::from_raw_fd(slave) };
        let master = unsafe { std::fs::File::from_raw_fd(master) };
        let mut command = Command::new(env!("CARGO_BIN_EXE_axiom-cli"));
        command
            .current_dir(directory)
            .arg("project")
            .args(args)
            .env_remove("CI")
            .env("TERM", "xterm")
            .env("AXIOM_CLOUD_URL", cloud)
            .env("AXIOM_LOCAL_PROFILE_DIR", directory)
            .env_remove("AXIOM_REFERRAL_CODE")
            .stdin(slave.try_clone().unwrap())
            .stdout(slave.try_clone().unwrap())
            .stderr(slave);
        // The terminal becomes the child's controlling terminal (`/dev/tty`).
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        // `command` holds the slave copies; drop them so reads end when the child exits.
        drop(command);
        Session { child, master, seen: String::new() }
    }

    impl Session {
        /// Read until `text` appears (true) or the terminal closes (false).
        fn until(&mut self, text: &str) -> bool {
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut buffer = [0u8; 4096];
            while !self.seen.contains(text) {
                let left = deadline.saturating_duration_since(Instant::now()).as_millis() as libc::c_int;
                let mut ready = libc::pollfd { fd: self.master.as_raw_fd(), events: libc::POLLIN, revents: 0 };
                let polled = unsafe { libc::poll(&mut ready, 1, left) };
                assert!(polled != 0 && left > 0, "timed out waiting for {text:?}; saw {:?}", self.seen);
                match self.master.read(&mut buffer) {
                    Ok(0) | Err(_) => return false,
                    Ok(n) => self.seen.push_str(&String::from_utf8_lossy(&buffer[..n])),
                }
            }
            true
        }
        fn type_line(&mut self, text: &str) {
            self.master.write_all(format!("{text}\r").as_bytes()).unwrap();
        }
        fn finish(mut self) -> (Option<i32>, String) {
            self.until("\u{0}never");
            (self.child.wait().unwrap().code(), std::mem::take(&mut self.seen))
        }
    }

    impl Drop for Session {
        /// A failed assertion must not leave the CLI waiting at its prompt.
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn methods(log: &Log) -> Vec<String> {
        log.lock().unwrap().iter().map(|r| format!("{} {}", r.method, r.path)).collect()
    }

    #[test]
    fn the_prompt_comes_before_any_request_and_the_phrase_then_archives() {
        let (cloud, log) = stub(route);
        let directory = tempfile::tempdir().unwrap();
        profile(directory.path());
        let mut session = spawn(&cloud, directory.path(), &["archive", "shop"]);
        assert!(session.until(PROMPT), "no prompt: {:?}", session.seen);
        assert!(session.seen.contains("Type `archive shop` to confirm"), "{:?}", session.seen);
        // Let a request that raced the prompt land before looking.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(methods(&log), Vec::<String>::new(), "requests before the prompt");
        session.type_line("archive shop");
        let (code, text) = session.finish();
        assert_eq!(code, Some(0), "{text}");
        assert!(text.contains("Archive of shop started"), "{text}");
        let sent = methods(&log);
        assert!(sent.contains(&"GET /api/v1/projects".to_string()), "{sent:?}");
        assert_eq!(sent.iter().filter(|r| r.starts_with("POST")).collect::<Vec<_>>(), vec!["POST /api/v1/projects/shop/archive"]);
    }

    #[test]
    fn a_wrong_phrase_sends_nothing() {
        let (cloud, log) = stub(route);
        let directory = tempfile::tempdir().unwrap();
        profile(directory.path());
        let mut session = spawn(&cloud, directory.path(), &["archive", "shop"]);
        assert!(session.until(PROMPT), "no prompt: {:?}", session.seen);
        session.type_line("archive Shop");
        let (code, text) = session.finish();
        assert_eq!(code, Some(2), "{text}");
        assert!(text.contains("axiom project archive: the confirmation did not match; nothing was sent"), "{text}");
        assert_eq!(methods(&log), Vec::<String>::new());
    }

    #[test]
    fn a_name_is_refused_unprompted_and_a_slug_of_another_project_posts_nothing() {
        let (cloud, log) = stub(route);
        let directory = tempfile::tempdir().unwrap();
        profile(directory.path());
        // `Shop` is the project's name, not a slug: refused before the prompt and any request.
        let session = spawn(&cloud, directory.path(), &["archive", "Shop"]);
        let (code, text) = session.finish();
        assert_eq!(code, Some(2), "{text}");
        assert!(text.contains("`Shop` is not a project slug") && !text.contains(PROMPT), "{text}");
        assert_eq!(methods(&log), Vec::<String>::new());

        // The project id is typed: confirmed, then looked up, then refused without a write.
        let mut session = spawn(&cloud, directory.path(), &["archive", PROJECT_ID]);
        assert!(session.until(PROMPT), "no prompt: {:?}", session.seen);
        assert_eq!(methods(&log), Vec::<String>::new(), "requests before the prompt");
        session.type_line(&format!("archive {PROJECT_ID}"));
        let (code, text) = session.finish();
        assert_eq!(code, Some(1), "{text}");
        assert!(text.contains(&format!("`{PROJECT_ID}` is the project `shop`. Nothing was archived.")), "{text}");
        let sent = methods(&log);
        assert!(!sent.is_empty() && sent.iter().all(|r| r.starts_with("GET ")), "{sent:?}");
    }
}
