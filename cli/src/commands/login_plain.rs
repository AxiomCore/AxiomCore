//! `axiom login` without the full-screen interface (B11, UX-9).
//!
//! The device flow is the same as in the TUI: start a device code, show the link
//! and the code, poll until the browser approves it. This module owns the parts
//! that must also work in CI, in a pipe and in a terminal that reports no size:
//!
//! * [`choose_mode`] decides between the TUI and plain text;
//! * [`start_device_login`] reads the device response, including the optional
//!   `verification_uri_complete` link with the code already filled in;
//! * [`PollState`] is the polling state machine (server interval, `slow_down`,
//!   transient failures) and [`poll_for_session`] drives it over HTTP;
//! * [`run_plain`] prints the link and code as plain lines and exits when done.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{IsTerminal, Write};
use std::time::Duration;

/// RFC 8628 §3.5: `slow_down` adds 5 seconds to the polling interval.
const SLOW_DOWN_STEP: u64 = 5;
/// Used when the server omits `interval` (RFC 8628 default).
const DEFAULT_INTERVAL: u64 = 5;
/// Used when the server omits `expires_in`.
const DEFAULT_EXPIRES_IN: u64 = 600;
/// Consecutive network or 5xx failures tolerated while waiting for approval.
const MAX_TRANSIENT_FAILURES: u32 = 3;
/// Smallest terminal the login screen can draw its link and code in.
const MIN_TUI_COLUMNS: u16 = 40;
const MIN_TUI_ROWS: u16 = 10;

// -----------------------------------------------------------------------------
// TUI or plain text
// -----------------------------------------------------------------------------

/// What the CLI knows about its terminal when `axiom login` starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalFacts {
    pub stdout_tty: bool,
    pub stdin_tty: bool,
    /// `(columns, rows)`, or `None` when the size cannot be read.
    pub size: Option<(u16, u16)>,
    /// `TERM=dumb`: no cursor control, so no alternate screen.
    pub dumb: bool,
}

impl TerminalFacts {
    pub fn detect() -> Self {
        let stdout_tty = std::io::stdout().is_terminal();
        Self {
            stdout_tty,
            stdin_tty: std::io::stdin().is_terminal(),
            // Only ask for a size when there is a terminal to ask.
            size: if stdout_tty {
                crossterm::terminal::size().ok()
            } else {
                None
            },
            dumb: std::env::var("TERM").is_ok_and(|term| term == "dumb"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginMode {
    Tui,
    Plain,
}

/// The TUI needs an interactive terminal it can measure and read keys from;
/// anything else (a pipe, CI, `script` without a size, `--no-tui`) gets plain text.
pub fn choose_mode(no_tui: bool, terminal: TerminalFacts) -> LoginMode {
    if no_tui || !terminal.stdout_tty || !terminal.stdin_tty || terminal.dumb {
        return LoginMode::Plain;
    }
    match terminal.size {
        Some((columns, rows)) if columns >= MIN_TUI_COLUMNS && rows >= MIN_TUI_ROWS => {
            LoginMode::Tui
        }
        _ => LoginMode::Plain,
    }
}

// -----------------------------------------------------------------------------
// Device start
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceStart {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    /// RFC 8628 §3.3.1: the verification link with the user code filled in.
    #[serde(default)]
    pub verification_uri_complete: Option<String>,
    #[serde(default)]
    pub interval: Option<u64>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

impl DeviceStart {
    /// The prefilled link when the server sent a usable one (UX-9).
    pub fn complete_link(&self) -> Option<&str> {
        self.verification_uri_complete
            .as_deref()
            .map(str::trim)
            .filter(|link| is_web_link(link))
    }

    /// The link to show and open: the prefilled one if present, else the bare URI.
    pub fn link(&self) -> &str {
        self.complete_link().unwrap_or(&self.verification_uri)
    }

    pub fn interval_seconds(&self) -> u64 {
        self.interval.unwrap_or(DEFAULT_INTERVAL)
    }

    pub fn expires_in_seconds(&self) -> u64 {
        self.expires_in.unwrap_or(DEFAULT_EXPIRES_IN)
    }
}

/// Only absolute http(s) links are shown as the prefilled link or handed to a browser.
fn is_web_link(value: &str) -> bool {
    url::Url::parse(value)
        .map(|link| matches!(link.scheme(), "http" | "https") && link.host_str().is_some())
        .unwrap_or(false)
}

pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?)
}

pub async fn start_device_login(client: &reqwest::Client, base_url: &str) -> Result<DeviceStart> {
    let base = base_url.trim_end_matches('/');
    let response = client
        .post(format!("{base}/auth/device/start"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .context("Could not contact AxiomCore authentication. Check your connection and retry")?;
    let status = response.status();
    if !status.is_success() {
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let body = response.text().await.unwrap_or_default();
        bail!(start_error(status.as_u16(), retry_after, &body));
    }
    let start: DeviceStart = response
        .json()
        .await
        .context("AxiomCore authentication returned an invalid device-login response")?;
    if start.device_code.trim().is_empty() || start.user_code.trim().is_empty() {
        bail!("AxiomCore authentication returned an invalid device-login response");
    }
    Ok(start)
}

/// A short, terminal-safe message for a failed device start (proxies may send HTML).
fn start_error(status: u16, retry_after: Option<u64>, body: &str) -> String {
    let code = error_code(body.as_bytes());
    let wait = retry_after
        .map(|seconds| format!(" Wait {seconds} seconds before retrying."))
        .unwrap_or_else(|| " Wait a minute before retrying.".to_string());
    if status == 429
        || code
            .as_deref()
            .is_some_and(|code| code.to_ascii_lowercase().contains("rate exceeded"))
    {
        return format!("Too many device-login attempts.{wait}");
    }
    if status == 503 {
        return "AxiomCore authentication is temporarily unavailable (HTTP 503). Please retry in about 30 seconds.".to_string();
    }
    if status >= 500 {
        return format!(
            "AxiomCore authentication is temporarily unavailable (HTTP {status}). Please retry shortly."
        );
    }
    match code.filter(|code| looks_like_code(code)) {
        Some(code) => format!("Could not start device login: {code}."),
        None => format!(
            "Could not start device login (HTTP {status}). Please retry or contact support if the problem persists."
        ),
    }
}

// -----------------------------------------------------------------------------
// Polling state machine
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub expires_in: u64,
}

/// One poll response, classified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    Pending,
    SlowDown,
    Approved(Session),
    Expired,
    Denied,
    /// Network error, 5xx: worth another try.
    Transient(String),
    Failed(String),
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// The device-flow error code in a poll body. The API sends JSON
/// (`{"error":"authorization_pending"}`) for a pending code but plain text
/// (`expired_token\n`) for the others, so both are accepted.
fn error_code(body: &[u8]) -> Option<String> {
    if let Ok(error) = serde_json::from_slice::<ErrorBody>(body) {
        return Some(error.error);
    }
    let text = std::str::from_utf8(body).ok()?.trim();
    matches!(
        text,
        "authorization_pending" | "slow_down" | "expired_token" | "access_denied"
    )
    .then(|| text.to_string())
}

/// Short identifiers are safe to echo; anything else (HTML, prose) is not.
fn looks_like_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' '))
}

pub fn classify(status: u16, body: &[u8]) -> PollOutcome {
    if let Some(code) = error_code(body) {
        return match code.as_str() {
            "authorization_pending" => PollOutcome::Pending,
            "slow_down" => PollOutcome::SlowDown,
            "expired_token" => PollOutcome::Expired,
            "access_denied" => PollOutcome::Denied,
            other if looks_like_code(other) => {
                PollOutcome::Failed(format!("Authentication failed: {other}."))
            }
            _ => PollOutcome::Failed(format!("Authentication failed (HTTP {status}).")),
        };
    }
    if status == 429 {
        return PollOutcome::SlowDown;
    }
    if status >= 500 {
        return PollOutcome::Transient(format!(
            "AxiomCore authentication is temporarily unavailable (HTTP {status})"
        ));
    }
    if !(200..300).contains(&status) {
        return PollOutcome::Failed(format!("Authentication failed (HTTP {status})."));
    }
    match serde_json::from_slice::<Session>(body) {
        Ok(session)
            if !session.access_token.trim().is_empty()
                && !session.refresh_token.trim().is_empty() =>
        {
            PollOutcome::Approved(session)
        }
        _ => PollOutcome::Failed(
            "AxiomCore authentication returned an invalid token response.".to_string(),
        ),
    }
}

/// What to do after a poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    /// Wait this many interval units, then poll again.
    Wait(u64),
    Done(std::result::Result<Session, String>),
}

pub const EXPIRED_MESSAGE: &str =
    "The login code expired before it was approved. Run `axiom login` again.";
pub const DENIED_MESSAGE: &str =
    "The sign-in was declined in the browser. Run `axiom login` to try again.";

/// The device-flow polling state: the current interval (server seconds, raised
/// by `slow_down`) and the count of consecutive transient failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollState {
    pub interval: u64,
    transient_failures: u32,
}

impl PollState {
    pub fn new(server_interval: u64) -> Self {
        Self {
            // A zero interval would poll in a tight loop.
            interval: server_interval.max(1),
            transient_failures: 0,
        }
    }

    pub fn next(&mut self, outcome: PollOutcome) -> Next {
        if !matches!(outcome, PollOutcome::Transient(_)) {
            self.transient_failures = 0;
        }
        match outcome {
            PollOutcome::Pending => Next::Wait(self.interval),
            PollOutcome::SlowDown => {
                self.interval = self.interval.saturating_add(SLOW_DOWN_STEP);
                Next::Wait(self.interval)
            }
            PollOutcome::Transient(message) => {
                self.transient_failures += 1;
                if self.transient_failures > MAX_TRANSIENT_FAILURES {
                    Next::Done(Err(format!("{message}. Run `axiom login` again.")))
                } else {
                    Next::Wait(self.interval)
                }
            }
            PollOutcome::Approved(session) => Next::Done(Ok(session)),
            PollOutcome::Expired => Next::Done(Err(EXPIRED_MESSAGE.to_string())),
            PollOutcome::Denied => Next::Done(Err(DENIED_MESSAGE.to_string())),
            PollOutcome::Failed(message) => Next::Done(Err(message)),
        }
    }
}

/// Wall-clock length of one interval second. Production is one second; tests
/// shrink it so the state machine runs against a real HTTP server quickly.
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub unit: Duration,
}

impl Timing {
    pub const PRODUCTION: Timing = Timing {
        unit: Duration::from_secs(1),
    };

    fn of(&self, units: u64) -> Duration {
        self.unit
            .saturating_mul(u32::try_from(units).unwrap_or(u32::MAX))
    }
}

/// Poll `/auth/device/poll` until the code is approved, declined or expired.
pub async fn poll_for_session(
    client: &reqwest::Client,
    base_url: &str,
    start: &DeviceStart,
    timing: Timing,
) -> Result<Session> {
    let base = base_url.trim_end_matches('/');
    let poll_url = format!("{base}/auth/device/poll");
    let deadline = tokio::time::Instant::now() + timing.of(start.expires_in_seconds());
    let mut state = PollState::new(start.interval_seconds());
    let mut wait = state.interval;
    loop {
        let wake = tokio::time::Instant::now() + timing.of(wait);
        if wake > deadline {
            bail!(EXPIRED_MESSAGE);
        }
        tokio::time::sleep_until(wake).await;
        let outcome = match client
            .post(&poll_url)
            .json(&serde_json::json!({ "device_code": start.device_code }))
            .send()
            .await
        {
            Ok(response) => {
                let status = response.status().as_u16();
                match response.bytes().await {
                    Ok(body) => classify(status, &body),
                    Err(_) => PollOutcome::Transient(
                        "Could not read the authentication response".to_string(),
                    ),
                }
            }
            Err(_) => PollOutcome::Transient(
                "Could not contact AxiomCore authentication while waiting for approval".to_string(),
            ),
        };
        match state.next(outcome) {
            Next::Wait(units) => wait = units,
            Next::Done(Ok(session)) => return Ok(session),
            Next::Done(Err(message)) => bail!(message),
        }
    }
}

/// Store a new CLI session the same way the TUI does.
pub fn save_session(session: &Session) -> Result<()> {
    crate::auth_store::save_tokens(
        &session.access_token,
        &session.refresh_token,
        session.expires_in,
    )?;
    crate::commands::step_up::StepUpStore::forget_for_profile();
    Ok(())
}

// -----------------------------------------------------------------------------
// Browser
// -----------------------------------------------------------------------------

/// Open the prefilled link only for a person at an interactive terminal, never
/// in CI or a pipe, and never for the bare URI (the behaviour before UX-9).
pub fn should_open_browser(start: &DeviceStart, terminal: TerminalFacts) -> bool {
    start.complete_link().is_some()
        && terminal.stdout_tty
        && terminal.stdin_tty
        && std::env::var_os("CI").is_none()
        && std::env::var_os("AXIOM_NO_BROWSER").is_none()
}

/// Best effort: a missing launcher is not an error, the link is printed anyway.
pub fn open_browser(link: &str) {
    let Ok(parsed) = url::Url::parse(link) else {
        return;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return;
    }
    let link = parsed.as_str();
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg(link);
        command
    } else if cfg!(target_os = "windows") {
        // Not `cmd /C start`: cmd would interpret `&` in the query string.
        let mut command = std::process::Command::new("rundll32");
        command.args(["url.dll,FileProtocolHandler", link]);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(link);
        command
    };
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Ok(mut child) = command.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

// -----------------------------------------------------------------------------
// Plain-text login
// -----------------------------------------------------------------------------

/// The lines plain mode prints before it starts polling.
pub fn instructions(start: &DeviceStart) -> Vec<String> {
    let minutes = start.expires_in_seconds().div_ceil(60).max(1);
    let mut lines = match start.complete_link() {
        Some(link) => vec![
            "To sign in to AxiomCore, open this link in a browser:".to_string(),
            format!("  {link}"),
            format!("Check that the page shows this code: {}", start.user_code),
        ],
        None => vec![
            "To sign in to AxiomCore, open this link in a browser:".to_string(),
            format!("  {}", start.verification_uri),
            format!("and enter this code: {}", start.user_code),
        ],
    };
    lines.push(format!(
        "Waiting for approval (the code expires in {minutes} minute{})...",
        if minutes == 1 { "" } else { "s" }
    ));
    lines
}

fn print_lines(lines: &[String]) {
    let mut stdout = std::io::stdout().lock();
    for line in lines {
        let _ = writeln!(stdout, "{line}");
    }
    let _ = stdout.flush();
}

/// Plain-text device login for a device code that was already started.
pub async fn run_plain_with(
    client: &reqwest::Client,
    base_url: &str,
    start: &DeviceStart,
    terminal: TerminalFacts,
) -> Result<()> {
    print_lines(&instructions(start));
    if should_open_browser(start, terminal) {
        open_browser(start.link());
    }
    let session = poll_for_session(client, base_url, start, Timing::PRODUCTION).await?;
    save_session(&session)?;
    print_lines(&["Signed in to AxiomCore.".to_string()]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{extract::State, routing::post, Json, Router};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    const TTY: TerminalFacts = TerminalFacts {
        stdout_tty: true,
        stdin_tty: true,
        size: Some((120, 40)),
        dumb: false,
    };

    // --- TUI or plain --------------------------------------------------------

    #[test]
    fn an_interactive_terminal_with_a_size_gets_the_tui() {
        assert_eq!(choose_mode(false, TTY), LoginMode::Tui);
        assert_eq!(
            choose_mode(
                false,
                TerminalFacts {
                    size: Some((MIN_TUI_COLUMNS, MIN_TUI_ROWS)),
                    ..TTY
                }
            ),
            LoginMode::Tui
        );
    }

    #[test]
    fn no_tui_flag_forces_plain_text() {
        assert_eq!(choose_mode(true, TTY), LoginMode::Plain);
    }

    #[test]
    fn a_pipe_gets_plain_text() {
        let piped = TerminalFacts {
            stdout_tty: false,
            size: None,
            ..TTY
        };
        assert_eq!(choose_mode(false, piped), LoginMode::Plain);
        let no_stdin = TerminalFacts {
            stdin_tty: false,
            ..TTY
        };
        assert_eq!(choose_mode(false, no_stdin), LoginMode::Plain);
    }

    #[test]
    fn a_terminal_without_a_size_gets_plain_text() {
        // `script` with no controlling terminal reports 0x0.
        for size in [
            Some((0, 0)),
            None,
            Some((0, 40)),
            Some((120, 0)),
            Some((30, 8)),
        ] {
            assert_eq!(
                choose_mode(false, TerminalFacts { size, ..TTY }),
                LoginMode::Plain,
                "{size:?}"
            );
        }
    }

    #[test]
    fn a_dumb_terminal_gets_plain_text() {
        assert_eq!(
            choose_mode(false, TerminalFacts { dumb: true, ..TTY }),
            LoginMode::Plain
        );
    }

    // --- UX-9 ----------------------------------------------------------------

    fn start(complete: Option<&str>) -> DeviceStart {
        DeviceStart {
            device_code: "dc-1".into(),
            user_code: "ABCD-EFGH".into(),
            verification_uri: "https://api.example.test/device".into(),
            verification_uri_complete: complete.map(str::to_string),
            interval: Some(1),
            expires_in: Some(600),
        }
    }

    #[test]
    fn the_prefilled_link_is_used_when_present() {
        let with = start(Some("https://api.example.test/device?user_code=ABCD-EFGH"));
        assert_eq!(
            with.link(),
            "https://api.example.test/device?user_code=ABCD-EFGH"
        );
        let lines = instructions(&with).join("\n");
        assert!(lines.contains("  https://api.example.test/device?user_code=ABCD-EFGH\n"));
        assert!(lines.contains("ABCD-EFGH"));
        assert!(!lines.contains("enter this code"));
    }

    #[test]
    fn without_a_prefilled_link_the_bare_uri_and_code_are_shown() {
        for complete in [
            None,
            Some(""),
            Some("  "),
            Some("javascript:alert(1)"),
            Some("/device"),
        ] {
            let bare = start(complete);
            assert_eq!(
                bare.link(),
                "https://api.example.test/device",
                "{complete:?}"
            );
            let lines = instructions(&bare).join("\n");
            assert!(lines.contains("  https://api.example.test/device\n"));
            assert!(lines.contains("and enter this code: ABCD-EFGH"));
        }
    }

    #[test]
    fn the_browser_opens_only_for_a_prefilled_link_at_a_terminal() {
        let complete = start(Some("https://api.example.test/device?user_code=ABCD-EFGH"));
        let piped = TerminalFacts {
            stdout_tty: false,
            ..TTY
        };
        assert!(!should_open_browser(&complete, piped));
        assert!(!should_open_browser(&start(None), TTY));
    }

    #[test]
    fn the_device_response_parses_with_and_without_the_prefilled_link() {
        let bare: DeviceStart = serde_json::from_str(
            r#"{"device_code":"d","user_code":"U","verification_uri":"https://x.test/device","expires_in":600,"interval":5}"#,
        )
        .unwrap();
        assert_eq!(bare.verification_uri_complete, None);
        assert_eq!(bare.interval_seconds(), 5);
        let complete: DeviceStart = serde_json::from_str(
            r#"{"device_code":"d","user_code":"U","verification_uri":"https://x.test/device","verification_uri_complete":"https://x.test/device?user_code=U"}"#,
        )
        .unwrap();
        assert_eq!(complete.link(), "https://x.test/device?user_code=U");
        assert_eq!(complete.interval_seconds(), DEFAULT_INTERVAL);
        assert_eq!(complete.expires_in_seconds(), DEFAULT_EXPIRES_IN);
    }

    // --- classification and state machine -------------------------------------

    fn session() -> Session {
        Session {
            access_token: "at".into(),
            refresh_token: "rt".into(),
            expires_in: 3600,
        }
    }

    #[test]
    fn poll_bodies_are_classified() {
        assert_eq!(
            classify(200, br#"{"error":"authorization_pending"}"#),
            PollOutcome::Pending
        );
        assert_eq!(
            classify(400, br#"{"error":"slow_down"}"#),
            PollOutcome::SlowDown
        );
        // The API answers expired codes with http.Error: plain text, HTTP 400.
        assert_eq!(classify(400, b"expired_token\n"), PollOutcome::Expired);
        assert_eq!(
            classify(400, br#"{"error":"access_denied"}"#),
            PollOutcome::Denied
        );
        assert_eq!(classify(429, b""), PollOutcome::SlowDown);
        assert!(matches!(
            classify(503, b"<html>503</html>"),
            PollOutcome::Transient(_)
        ));
        assert_eq!(
            classify(
                200,
                br#"{"access_token":"at","refresh_token":"rt","expires_in":3600}"#
            ),
            PollOutcome::Approved(session())
        );
        assert!(matches!(
            classify(200, br#"{"access_token":"","refresh_token":"rt"}"#),
            PollOutcome::Failed(_)
        ));
        match classify(400, b"<html><body>bad</body></html>") {
            PollOutcome::Failed(message) => {
                assert!(!message.contains("<html>"), "{message}");
                assert!(message.contains("HTTP 400"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_state_machine_follows_the_server_interval_and_slow_down() {
        let mut state = PollState::new(5);
        assert_eq!(state.next(PollOutcome::Pending), Next::Wait(5));
        assert_eq!(state.next(PollOutcome::SlowDown), Next::Wait(10));
        assert_eq!(state.next(PollOutcome::Pending), Next::Wait(10));
        assert_eq!(state.next(PollOutcome::SlowDown), Next::Wait(15));
        assert_eq!(
            state.next(PollOutcome::Approved(session())),
            Next::Done(Ok(session()))
        );
        // A zero interval would poll in a tight loop.
        assert_eq!(PollState::new(0).interval, 1);
    }

    #[test]
    fn the_state_machine_ends_on_expiry_and_denial() {
        assert_eq!(
            PollState::new(5).next(PollOutcome::Expired),
            Next::Done(Err(EXPIRED_MESSAGE.to_string()))
        );
        assert_eq!(
            PollState::new(5).next(PollOutcome::Denied),
            Next::Done(Err(DENIED_MESSAGE.to_string()))
        );
    }

    #[test]
    fn transient_failures_are_retried_a_few_times_in_a_row() {
        let mut state = PollState::new(2);
        let blip = || PollOutcome::Transient("HTTP 503".into());
        for _ in 0..MAX_TRANSIENT_FAILURES {
            assert_eq!(state.next(blip()), Next::Wait(2));
        }
        // A good answer resets the count.
        assert_eq!(state.next(PollOutcome::Pending), Next::Wait(2));
        for _ in 0..MAX_TRANSIENT_FAILURES {
            assert_eq!(state.next(blip()), Next::Wait(2));
        }
        assert!(matches!(state.next(blip()), Next::Done(Err(_))));
    }

    // --- against a mock HTTP server ---------------------------------------------

    #[derive(Clone)]
    struct Script {
        polls: Arc<Mutex<Vec<(u16, String)>>>,
        seen: Arc<Mutex<Vec<(Instant, String)>>>,
    }

    async fn poll(
        State(script): State<Script>,
        Json(body): Json<serde_json::Value>,
    ) -> (axum::http::StatusCode, String) {
        script.seen.lock().unwrap().push((
            Instant::now(),
            body["device_code"].as_str().unwrap_or_default().to_string(),
        ));
        let mut polls = script.polls.lock().unwrap();
        let (status, body) = if polls.is_empty() {
            (200, r#"{"error":"authorization_pending"}"#.to_string())
        } else {
            polls.remove(0)
        };
        (axum::http::StatusCode::from_u16(status).unwrap(), body)
    }

    async fn device_start() -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "device_code": "dc-1",
            "user_code": "ABCD-EFGH",
            "verification_uri": "https://api.example.test/device",
            "verification_uri_complete": "https://api.example.test/device?user_code=ABCD-EFGH",
            "expires_in": 600,
            "interval": 2,
        }))
    }

    async fn mock(polls: Vec<(u16, &str)>) -> (String, Script) {
        let script = Script {
            polls: Arc::new(Mutex::new(
                polls.into_iter().map(|(s, b)| (s, b.to_string())).collect(),
            )),
            seen: Arc::default(),
        };
        let app = Router::new()
            .route("/auth/device/start", post(device_start))
            .route("/auth/device/poll", post(poll))
            .with_state(script.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, script)
    }

    const UNIT: Duration = Duration::from_millis(40);

    #[tokio::test]
    async fn pending_then_slow_down_then_success() {
        let (base, script) = mock(vec![
            (200, r#"{"error":"authorization_pending"}"#),
            (400, r#"{"error":"slow_down"}"#),
            (200, r#"{"error":"authorization_pending"}"#),
            (
                200,
                r#"{"access_token":"at","refresh_token":"rt","expires_in":3600}"#,
            ),
        ])
        .await;
        let client = http_client().unwrap();
        let start = start_device_login(&client, &base).await.unwrap();
        assert_eq!(
            start.link(),
            "https://api.example.test/device?user_code=ABCD-EFGH"
        );
        let began = Instant::now();
        let session = poll_for_session(&client, &base, &start, Timing { unit: UNIT })
            .await
            .unwrap();
        assert_eq!(session, self::session());

        let seen = script.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 4);
        assert!(seen.iter().all(|(_, code)| code == "dc-1"));
        // Gaps: interval 2, 2, then 7 after slow_down, 7.
        let gaps: Vec<Duration> = std::iter::once(seen[0].0 - began)
            .chain(seen.windows(2).map(|pair| pair[1].0 - pair[0].0))
            .collect();
        for (gap, units) in gaps.iter().zip([2u32, 2, 7, 7]) {
            assert!(*gap >= UNIT * units, "gap {gap:?} < {units} units");
        }
        assert!(
            gaps[2] > gaps[1] + UNIT * 4,
            "slow_down did not widen: {gaps:?}"
        );
    }

    #[tokio::test]
    async fn an_expired_code_ends_the_login() {
        let (base, _script) = mock(vec![
            (200, r#"{"error":"authorization_pending"}"#),
            (400, "expired_token\n"),
        ])
        .await;
        let client = http_client().unwrap();
        let start = start_device_login(&client, &base).await.unwrap();
        let error = poll_for_session(&client, &base, &start, Timing { unit: UNIT })
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), EXPIRED_MESSAGE);
    }

    #[tokio::test]
    async fn the_local_deadline_ends_a_code_that_stays_pending() {
        let (base, script) = mock(vec![]).await;
        let client = http_client().unwrap();
        let start = DeviceStart {
            interval: Some(1),
            expires_in: Some(3),
            ..start_device_login(&client, &base).await.unwrap()
        };
        let error = poll_for_session(&client, &base, &start, Timing { unit: UNIT })
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), EXPIRED_MESSAGE);
        // Polls at about 1, 2 (and 3) units; never after the code's lifetime.
        let polls = script.seen.lock().unwrap().len();
        assert!((2..=3).contains(&polls), "{polls} polls");
    }

    #[tokio::test]
    async fn a_rate_limited_start_is_actionable() {
        let app = Router::new().route(
            "/auth/device/start",
            post(|| async {
                (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    [("retry-after", "30")],
                    r#"{"error":"Rate exceeded"}"#,
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let error = start_device_login(&http_client().unwrap(), &base)
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Too many device-login attempts. Wait 30 seconds before retrying."
        );
    }
}
