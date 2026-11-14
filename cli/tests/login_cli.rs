//! `axiom login` without a usable terminal (B11) against a loopback device-flow stub:
//! stdout piped, `script` with a 0x0 terminal, and `--no-tui` on a sized terminal.
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const COMPLETE: &str = "https://api.example.test/device?user_code=WXYZ-2345";
const BARE: &str = "https://api.example.test/device";
const CODE: &str = "WXYZ-2345";
const TOKEN: &str = r#"{"access_token":"login-access-secret","refresh_token":"login-refresh-secret","expires_in":3600}"#;

type Polls = Arc<Mutex<Vec<(u16, String)>>>;

/// Serve `/auth/device/start` with `start` and answer each `/auth/device/poll`
/// with the next scripted response. Returns the base URL and the poll count.
fn stub(start: String, polls: Vec<(u16, &str)>) -> (String, Arc<Mutex<usize>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let polls: Polls = Arc::new(Mutex::new(
        polls.into_iter().map(|(s, b)| (s, b.to_string())).collect(),
    ));
    let count = Arc::new(Mutex::new(0usize));
    let counted = count.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let mut length = 0usize;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).is_err() || header == "\r\n" || header.is_empty() {
                    break;
                }
                if let Some((name, value)) = header.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            let (status, response) = match path.as_str() {
                "/auth/device/start" => (200, start.clone()),
                "/auth/device/poll" => {
                    *counted.lock().unwrap() += 1;
                    let mut polls = polls.lock().unwrap();
                    if polls.is_empty() {
                        (200, r#"{"error":"authorization_pending"}"#.to_string())
                    } else {
                        polls.remove(0)
                    }
                }
                _ => (404, "{}".to_string()),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                response.len()
            );
        }
    });
    (address, count)
}

fn start_body(complete: bool) -> String {
    let mut body = serde_json::json!({
        "device_code": "device-secret",
        "user_code": CODE,
        "verification_uri": BARE,
        "expires_in": 600,
        "interval": 1,
    });
    if complete {
        body["verification_uri_complete"] = COMPLETE.into();
    }
    body.to_string()
}

fn command(program: &str, cloud: &str, profile: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .current_dir(profile)
        .env("CI", "true")
        .env("HOME", profile)
        .env("XDG_CONFIG_HOME", profile)
        .env("AXIOM_CLOUD_URL", cloud)
        .env("AXIOM_LOCAL_PROFILE_DIR", profile)
        .env("AXIOM_NO_BROWSER", "1")
        .env_remove("AXIOM_REFERRAL_CODE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Run to completion, but fail instead of hanging (the 0x0 terminal used to hang).
fn run(command: &mut Command) -> Output {
    let mut child = command.spawn().unwrap();
    let began = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if began.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "axiom login did not finish within 60 s\nstdout:\n{}\nstderr:\n{}",
                text(&output.stdout),
                text(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    child.wait_with_output().unwrap()
}

fn login_piped(cloud: &str, profile: &Path) -> Output {
    run(command(env!("CARGO_BIN_EXE_axiom-cli"), cloud, profile).arg("login"))
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

fn assert_signed_in(output: &Output, profile: &Path) {
    let stdout = text(&output.stdout);
    let stderr = text(&output.stderr);
    assert!(
        output.status.success(),
        "stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("Signed in to AxiomCore."), "{stdout}");
    for secret in [
        "login-access-secret",
        "login-refresh-secret",
        "device-secret",
    ] {
        assert!(
            !stdout.contains(secret) && !stderr.contains(secret),
            "{secret} printed"
        );
    }
    // No alternate screen or raw-mode escape sequences in plain mode.
    assert!(
        !stdout.contains("\u{1b}[?1049h"),
        "TUI escape codes in {stdout:?}"
    );
    let saved = std::fs::read_to_string(profile.join("auth.json")).unwrap();
    let saved: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["access_token"], "login-access-secret");
    assert_eq!(saved["refresh_token"], "login-refresh-secret");
}

#[test]
fn piped_login_prints_the_prefilled_link_and_code_and_signs_in() {
    let profile = tempfile::tempdir().unwrap();
    let (cloud, polls) = stub(
        start_body(true),
        vec![(200, r#"{"error":"authorization_pending"}"#), (200, TOKEN)],
    );
    let output = login_piped(&cloud, profile.path());
    assert_signed_in(&output, profile.path());
    let stdout = text(&output.stdout);
    assert!(stdout.contains(&format!("  {COMPLETE}\n")), "{stdout}");
    assert!(stdout.contains(CODE), "{stdout}");
    assert_eq!(*polls.lock().unwrap(), 2);
}

#[test]
fn piped_login_without_a_prefilled_link_prints_the_bare_uri_and_code() {
    let profile = tempfile::tempdir().unwrap();
    let (cloud, _) = stub(start_body(false), vec![(200, TOKEN)]);
    let output = login_piped(&cloud, profile.path());
    assert_signed_in(&output, profile.path());
    let stdout = text(&output.stdout);
    assert!(stdout.contains(&format!("  {BARE}\n")), "{stdout}");
    assert!(
        stdout.contains(&format!("and enter this code: {CODE}")),
        "{stdout}"
    );
    assert!(!stdout.contains("user_code="), "{stdout}");
}

#[test]
fn piped_login_with_an_expired_code_fails_with_a_next_step() {
    let profile = tempfile::tempdir().unwrap();
    // The API sends expired codes as plain text with HTTP 400.
    let (cloud, _) = stub(start_body(true), vec![(400, "expired_token\n")]);
    let output = login_piped(&cloud, profile.path());
    let stderr = text(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("The login code expired before it was approved. Run `axiom login` again."),
        "{stderr}"
    );
    assert!(!profile.path().join("auth.json").exists());
}

/// `script` gives the CLI a pseudo-terminal. Run from a test (no controlling
/// terminal) it has no size: the 0x0 case that drew nothing before B11.
fn under_script(cloud: &str, profile: &Path, shell: &str) -> Option<Output> {
    let available = Command::new("script")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !available || !cfg!(target_os = "linux") {
        eprintln!("skipping: util-linux script is not available");
        return None;
    }
    Some(run(command("script", cloud, profile).args([
        "-q",
        "-e",
        "-c",
        shell,
        "/dev/null",
    ])))
}

#[test]
fn login_in_a_terminal_without_a_size_falls_back_to_plain_text() {
    let profile = tempfile::tempdir().unwrap();
    let (cloud, _) = stub(start_body(true), vec![(200, TOKEN)]);
    let binary = env!("CARGO_BIN_EXE_axiom-cli");
    let Some(output) = under_script(
        &cloud,
        profile.path(),
        &format!("stty cols 0 rows 0; '{binary}' login"),
    ) else {
        return;
    };
    assert_signed_in(&output, profile.path());
    let stdout = text(&output.stdout);
    assert!(stdout.contains(COMPLETE), "{stdout}");
    assert!(stdout.contains(CODE), "{stdout}");
}

#[test]
fn no_tui_forces_plain_text_on_a_sized_terminal() {
    let profile = tempfile::tempdir().unwrap();
    let (cloud, _) = stub(start_body(true), vec![(200, TOKEN)]);
    let binary = env!("CARGO_BIN_EXE_axiom-cli");
    let Some(output) = under_script(
        &cloud,
        profile.path(),
        &format!("stty cols 120 rows 40; '{binary}' login --no-tui"),
    ) else {
        return;
    };
    assert_signed_in(&output, profile.path());
    assert!(text(&output.stdout).contains(COMPLETE));
}
