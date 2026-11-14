//! Recovery commands (recovery/v1, API, Authorization and CLI sections):
//! `axiom project resources recovery` and the `request` actions `restore`,
//! `archive`, `unarchive` (shown as "resume") and `retire_previous`.
//!
//! The transport is separate from `axiom_cloud::CloudClient::resource_request`
//! because these actions need the response code (step-up on
//! `AXRESOURCE_REAUTH_REQUIRED`, `resumable`, `Retry-After`) and a second
//! bearer (the step-up token, which is never refreshed). Only known codes
//! select fixed messages; server messages and unknown values are never
//! echoed.

use crate::commands::step_up::{step_up, StepUpStore};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

/// The `request` actions this module owns.
pub const RECOVERY_ACTIONS: [&str; 4] = ["restore", "archive", "unarchive", "retire_previous"];

/// Moves: prepare and validate have a 30-minute deadline and the fence at
/// most 300 s; archive fences, backs up and deletes. Poll a little longer.
const MOVE_WAIT: Duration = Duration::from_secs(40 * 60);
const SHORT_WAIT: Duration = Duration::from_secs(5 * 60);
const MAX_ERROR_BODY: usize = 8192;
const MAX_BODY: usize = 2 * 1024 * 1024;

pub fn is_recovery_action(action: &str) -> bool {
    RECOVERY_ACTIONS.contains(&action)
}

/// The label people see: `unarchive` is "resume" (C-5: the `resume` action
/// keeps its meaning, un-suspend).
pub fn action_label(action: &str) -> &'static str {
    match action {
        "restore" => "restore",
        "archive" => "archive",
        "unarchive" => "resume",
        "retire_previous" => "retire previous generation",
        _ => "request",
    }
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

struct Session {
    access: String,
    refresh: String,
}

pub struct Api {
    base: String,
    http: reqwest::Client,
    session: tokio::sync::Mutex<Session>,
    persist_session: bool,
    step_up: StepUpStore,
    step_up_poll_floor: Duration,
    pub(crate) wait_interval: Duration,
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub body: Value,
    pub retry_after: Option<u64>,
}

impl Reply {
    fn code(&self) -> Option<&str> {
        self.body.get("code").and_then(Value::as_str)
    }
}

#[derive(Clone, Copy)]
enum ResponseKind {
    ResourceJson,
    DataJson,
    Csv,
}

impl Api {
    pub fn new(
        base: &str,
        access: String,
        refresh: String,
        persist_session: bool,
        step_up: StepUpStore,
    ) -> Result<Self> {
        Ok(Self {
            base: base.trim_end_matches('/').to_string(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(20))
                .build()?,
            session: tokio::sync::Mutex::new(Session { access, refresh }),
            persist_session,
            step_up,
            step_up_poll_floor: Duration::from_secs(1),
            wait_interval: Duration::from_secs(5),
        })
    }

    pub fn from_profile() -> Result<Self> {
        let data = crate::auth_store::load_auth_data()
            .context("Not logged in. Run 'axiom login' first.")?;
        if data.access_token.trim().is_empty() || data.refresh_token.trim().is_empty() {
            bail!("Not logged in. Run 'axiom login' first.");
        }
        Self::new(
            &axiom_cloud::cloud_base_url()?,
            data.access_token.clone(),
            data.refresh_token.clone(),
            true,
            StepUpStore::for_profile()?,
        )
    }

    #[cfg(test)]
    pub(crate) fn fast(mut self) -> Self {
        self.step_up_poll_floor = Duration::ZERO;
        self.wait_interval = Duration::from_millis(10);
        self
    }

    fn url(&self, segments: &[&str]) -> Result<reqwest::Url> {
        let mut url = reqwest::Url::parse(&self.base)?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|_| anyhow!("Invalid resource endpoint"))?;
            path.clear();
            path.extend(["api", "v1"]);
            path.extend(segments.iter().copied());
        }
        Ok(url)
    }

    async fn send(
        &self,
        method: &Method,
        url: reqwest::Url,
        body: Option<&Value>,
        key: Option<&str>,
        bearer: &str,
    ) -> Result<Reply> {
        self.send_as(method, url, body, key, bearer, ResponseKind::ResourceJson)
            .await
    }

    async fn send_as(
        &self,
        method: &Method,
        url: reqwest::Url,
        body: Option<&Value>,
        key: Option<&str>,
        bearer: &str,
        kind: ResponseKind,
    ) -> Result<Reply> {
        let mut request = self
            .http
            .request(method.clone(), url)
            .bearer_auth(bearer)
            .header(
                reqwest::header::ACCEPT,
                if matches!(kind, ResponseKind::Csv) {
                    "text/csv"
                } else {
                    "application/json"
                },
            );
        if !matches!(kind, ResponseKind::ResourceJson) {
            request = request.timeout(Duration::from_secs(140));
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(key) = key {
            request = request.header("Idempotency-Key", key);
        }
        let mut response = request.send().await.map_err(|_| {
            anyhow!("AXRESOURCE_UNAVAILABLE: request failed. Check your connection and retry.")
        })?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let csv = matches!(kind, ResponseKind::Csv) && (200..300).contains(&status);
        if csv
            && !response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/csv"))
        {
            bail!("AXDATA_INVALID_RESPONSE: expected a CSV export");
        }
        let limit = if (200..300).contains(&status) {
            match kind {
                ResponseKind::ResourceJson => MAX_BODY,
                ResponseKind::DataJson => 4 * 1024 * 1024,
                ResponseKind::Csv => 25 * 1024 * 1024,
            }
        } else {
            MAX_ERROR_BODY
        };
        let mut bytes = Vec::new();
        let mut complete = true;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("AXRESOURCE_UNAVAILABLE: response interrupted"))?
        {
            if bytes.len() + chunk.len() > limit {
                complete = false;
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        if !complete && (200..300).contains(&status) {
            bail!("AXRESOURCE_RESPONSE_TOO_LARGE");
        }
        let body = if csv {
            Value::String(
                String::from_utf8(bytes.clone())
                    .map_err(|_| anyhow!("AXDATA_INVALID_RESPONSE: export is not UTF-8"))?,
            )
        } else if complete {
            match serde_json::from_slice(&bytes) {
                Ok(value) => value,
                Err(_)
                    if matches!(kind, ResponseKind::DataJson)
                        && (200..300).contains(&status)
                        && status != 204 =>
                {
                    bail!("AXDATA_INVALID_RESPONSE: expected JSON");
                }
                Err(_) => Value::Null,
            }
        } else {
            Value::Null
        };
        bytes.fill(0);
        Ok(Reply {
            status,
            body,
            retry_after,
        })
    }

    /// Send with the CLI session, refreshing it once on 401.
    async fn send_session(
        &self,
        method: &Method,
        url: reqwest::Url,
        body: Option<&Value>,
        key: Option<&str>,
    ) -> Result<Reply> {
        self.send_session_as(method, url, body, key, ResponseKind::ResourceJson)
            .await
    }

    async fn send_session_as(
        &self,
        method: &Method,
        url: reqwest::Url,
        body: Option<&Value>,
        key: Option<&str>,
        kind: ResponseKind,
    ) -> Result<Reply> {
        let access = self.session.lock().await.access.clone();
        let reply = self
            .send_as(method, url.clone(), body, key, &access, kind)
            .await?;
        if reply.status != 401 {
            return Ok(reply);
        }
        let refresh = self.session.lock().await.refresh.clone();
        let tokens = axiom_cloud::auth::refresh_access_token(&self.http, &self.base, &refresh)
            .await
            .map_err(|_| {
                anyhow!("Your AxiomCore session has expired. Run 'axiom login' and retry.")
            })?;
        {
            let mut session = self.session.lock().await;
            session.access = tokens.access_token.clone();
            session.refresh = tokens.refresh_token.clone();
        }
        if self.persist_session {
            crate::auth_store::save_tokens(
                &tokens.access_token,
                &tokens.refresh_token,
                tokens.expires_in,
            )?;
        }
        self.send_as(method, url, body, key, &tokens.access_token, kind)
            .await
    }

    /// Data API request using the normal CLI session. Data errors expose only
    /// validated protocol codes, never SQL, values or engine error messages.
    /// A browser-only refusal remains a refusal; it is not a step-up challenge.
    pub async fn data_request(
        &self,
        method: &Method,
        segments: &[&str],
        body: Option<&Value>,
        key: Option<&str>,
        query: &[(&str, String)],
    ) -> Result<Value> {
        let mut url = self.url(segments)?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let mut reply = self
            .send_session_as(method, url.clone(), body, key, ResponseKind::DataJson)
            .await?;
        let mut elevated = None;
        if reply.status == 403 && reply.code() == Some("AXRESOURCE_REAUTH_REQUIRED") {
            // Current /data routes never issue this challenge. Keep the established
            // flow for forward compatibility, only on an explicit reauth code.
            step_up(&self.base, &self.step_up, self.step_up_poll_floor).await?;
            let token = self
                .step_up
                .load()
                .context("The step-up sign-in expired; retry the command")?;
            reply = self
                .send_as(method, url.clone(), body, key, &token, ResponseKind::DataJson)
                .await?;
            elevated = Some(token);
        }
        for _ in 0..3 {
            if key.is_none() || reply.status != 409 || reply.body["error"]["code"].as_str().or_else(|| reply.code()) != Some("AXDATA_WRITE_PENDING") { break; }
            tokio::time::sleep(Duration::from_secs(reply.retry_after.unwrap_or(1).min(10)).min(self.wait_interval)).await;
            reply = if let Some(token) = elevated.as_deref() {
                self.send_as(method, url.clone(), body, key, token, ResponseKind::DataJson).await?
            } else {
                self.send_session_as(method, url.clone(), body, key, ResponseKind::DataJson).await?
            };
        }
        data_route_result(reply, method, segments)
    }

    /// Export through the same session refresh and bounded transport as JSON.
    /// The CSV end marker is mandatory, including for truncated exports.
    pub async fn data_export(&self, segments: &[&str], body: &Value) -> Result<String> {
        let reply = self
            .send_session_as(
                &Method::POST,
                self.url(segments)?,
                Some(body),
                None,
                ResponseKind::Csv,
            )
            .await?;
        let value = data_route_result(reply, &Method::POST, segments)?;
        let text = value
            .as_str()
            .context("AXDATA_INVALID_RESPONSE: expected CSV")?;
        let last = text
            .strip_suffix("\r\n")
            .or_else(|| text.strip_suffix('\n'))
            .unwrap_or(text)
            .rsplit('\n')
            .next()
            .unwrap_or("")
            .trim_end_matches('\r');
        let marker = regex::Regex::new(r"^# (complete|truncated) (0|[1-9][0-9]{0,8}) rows$")?;
        ensure_data(
            marker.is_match(last),
            "AXDATA_INVALID_RESPONSE: export end marker is missing",
        )?;
        let expected = marker.captures(last).and_then(|c| c[2].parse::<usize>().ok()).context("Invalid export count")?;
        let mut quoted = false;
        let mut records = 0usize;
        for c in text.chars() {
            if c == '"' { quoted = !quoted; }
            else if c == '\n' && !quoted { records += 1; }
        }
        if !text.is_empty() && !text.ends_with('\n') { records += 1; }
        ensure_data(!quoted && records.checked_sub(2) == Some(expected), "AXDATA_INVALID_RESPONSE: export row count does not match")?;
        Ok(text.to_string())
    }

    /// A write with the CLI session only (no step-up), refreshed once on 401.
    pub async fn write(&self, method: &Method, segments: &[&str], body: &Value, key: Option<&str>) -> Result<Value> {
        let reply = self.send_session(method, self.url(segments)?, Some(body), key).await?;
        if (200..300).contains(&reply.status) {
            return Ok(reply.body);
        }
        Err(recovery_error(&reply))
    }

    /// A read with the CLI session, with a query string.
    pub async fn get_query(&self, segments: &[&str], query: &[(&str, String)]) -> Result<Value> {
        let mut url = self.url(segments)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let reply = self.send_session(&Method::GET, url, None, None).await?;
        if (200..300).contains(&reply.status) {
            return Ok(reply.body);
        }
        Err(recovery_error(&reply))
    }

    /// A read with the CLI session.
    pub async fn get(&self, segments: &[&str]) -> Result<Value> {
        let reply = self
            .send_session(&Method::GET, self.url(segments)?, None, None)
            .await?;
        if (200..300).contains(&reply.status) {
            return Ok(reply.body);
        }
        Err(recovery_error(&reply))
    }

    /// A recovery action. It uses a stored step-up token when one is valid,
    /// otherwise the session. On `403 AXRESOURCE_REAUTH_REQUIRED` (or a 401 on
    /// the step-up token, which is never refreshed) it runs the step-up
    /// device flow once and retries with the same Idempotency-Key.
    pub async fn post_action(&self, segments: &[&str], body: &Value, key: &str) -> Result<Value> {
        self.step_up_action(&Method::POST, segments, body, key).await
    }

    /// [`Api::post_action`] with any write method (placement `PUT …/connection`).
    pub async fn step_up_action(&self, method: &Method, segments: &[&str], body: &Value, key: &str) -> Result<Value> {
        self.step_up_action_query(method, segments, &[], body, key).await
    }

    /// [`Api::step_up_action`] with a query string (branch policy and settings).
    /// A write with the normal session only (no step-up token): for routes that need ownership but
    /// not a fresh sign-in, where the server refuses a step-up token (W8 QJ, BR branch settings).
    pub async fn session_action_query(&self, method: &Method, segments: &[&str], query: &[(&str, String)], body: &Value, key: &str) -> Result<Value> {
        let mut url = self.url(segments)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let reply = self.send_session(method, url, Some(body), Some(key)).await?;
        if (200..300).contains(&reply.status) {
            return Ok(reply.body);
        }
        Err(recovery_error(&reply))
    }

    pub async fn step_up_action_query(&self, method: &Method, segments: &[&str], query: &[(&str, String)], body: &Value, key: &str) -> Result<Value> {
        let mut url = self.url(segments)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        let mut stepped_up = false;
        loop {
            let step_up_token = self.step_up.load();
            let reply = match step_up_token.as_deref() {
                Some(token) => {
                    self.send(method, url.clone(), Some(body), Some(key), token)
                        .await?
                }
                None => {
                    self.send_session(method, url.clone(), Some(body), Some(key))
                        .await?
                }
            };
            if (200..300).contains(&reply.status) {
                return Ok(reply.body);
            }
            let reauth = reply.status == 403 && reply.code() == Some("AXRESOURCE_REAUTH_REQUIRED");
            let stale_step_up = step_up_token.is_some() && reply.status == 401;
            if (reauth || stale_step_up) && !stepped_up {
                self.step_up.clear();
                step_up(&self.base, &self.step_up, self.step_up_poll_floor).await?;
                stepped_up = true;
                continue;
            }
            if stale_step_up {
                self.step_up.clear();
                bail!("AXRESOURCE_REAUTH_REQUIRED: the step-up sign-in was not accepted. Run `axiom login --step-up` and retry.");
            }
            return Err(recovery_error(&reply));
        }
    }
}


fn ensure_data(valid: bool, message: &str) -> Result<()> {
    if !valid {
        bail!("{message}");
    }
    Ok(())
}

fn data_route_result(reply: Reply, method: &Method, segments: &[&str]) -> Result<Value> {
    if let ["projects", project, "data", "environments", environment, rest @ ..] = segments {
        let browser_only = (*method == Method::POST
            && matches!(rest, ["sql", "write", "preview" | "execute"]
                | ["tables", _, "rows:write" | "export"]
                | ["capture", "preview"] | ["capture"]))
            || (*method == Method::PUT && rest == ["capture", "settings"]);
        let error = &reply.body["error"];
        let reason = error.get("reason").or_else(|| reply.body.get("reason"));
        if reply.status == 403
            && error["code"].as_str().or_else(|| reply.code()) == Some("AXDATA_FORBIDDEN")
            && (reason.and_then(Value::as_str) == Some("browser_session")
                || (reason.is_none() && browser_only))
        {
            bail!("AXDATA_FORBIDDEN: This operation currently needs a dashboard browser session. Open https://app.axiomcore.dev/dashboard and select project {project}, environment {environment}, to do it.");
        }
    }
    data_result(reply)
}

fn data_result(reply: Reply) -> Result<Value> {
    if (200..300).contains(&reply.status) {
        return Ok(reply.body);
    }
    let code = reply
        .body
        .get("error")
        .and_then(|e| e.get("code"))
        .and_then(Value::as_str)
        .or_else(|| reply.code());
    if let Some(code) = code.filter(|c| {
        c.strip_prefix("AXDATA_").is_some_and(|tail| {
            !tail.is_empty()
                && tail.len() <= 40
                && tail.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
        })
    }) {
        bail!("{code}: data request refused (HTTP {}).", reply.status);
    }
    if reply.code() == Some("AXRESOURCE_REAUTH_REQUIRED") {
        bail!("AXRESOURCE_REAUTH_REQUIRED: the step-up sign-in was not accepted");
    }
    bail!(
        "AXDATA_REQUEST_FAILED: data request refused (HTTP {}).",
        reply.status
    )
}

// ---------------------------------------------------------------------------
// Codes
// ---------------------------------------------------------------------------

/// Plain words for every code of recovery/v1 and the existing resource codes.
pub fn code_message(
    code: &str,
    resumable: Option<bool>,
    retry_after: Option<u64>,
) -> Option<String> {
    let retry = retry_after
        .map(|seconds| format!(" Retry in {seconds} seconds."))
        .unwrap_or_else(|| " Retry in about 30 seconds.".to_string());
    Some(match code {
        "AXRESOURCE_MOVING" => "The database is switching to a restored or resumed copy. Requests pause for about two minutes; signed-in users stay signed in. Retry shortly.".to_string(),
        // W6-F L-21: a resume on next use is promised only when the server
        // says so (an inactivity archive).
        "AXRESOURCE_ARCHIVED" => match resumable {
            Some(true) => "The database is archived. It resumes on next use, or an Owner can resume it now with `--action unarchive`.".to_string(),
            Some(false) => "The database was archived by an Owner and does not resume on use. An Owner can resume it with `--action unarchive`.".to_string(),
            None => "The database is archived. An Owner can resume it with `--action unarchive`.".to_string(),
        },
        "AXRESOURCE_RESUMING" => format!("The database is resuming from its archive.{retry}"),
        "AXRESOURCE_POINT_UNAVAILABLE" => "That point is outside the restore window. Run `axiom project resources recovery` to see the earliest point, or restore from a daily backup with `--backup`.".to_string(),
        "AXRESOURCE_CAPACITY" => "Hosting capacity, the operation allowance, the daily restore limit or the plan's compute budget does not allow this now. Retry later.".to_string(),
        "AXRESOURCE_RESIDENCY" => "Backups are not available in this database's region yet, so this action cannot run.".to_string(),
        "AXRESOURCE_HEAD_UNKNOWN" => "AxiomCore has no record of the schema at that point. Choose a later point or a daily backup.".to_string(),
        "AXRESOURCE_ARTIFACT_REFUSED" => "The saved copy contains objects a restore does not accept. Choose another point or backup, or contact support.".to_string(),
        "AXRESOURCE_PREVIOUS_RETAINED" => "The state from before the last restore is still kept (24 hours). Wait until it is retired, or retire it now with `--action retire_previous`.".to_string(),
        "AXRESOURCE_REAUTH_REQUIRED" => "This action needs a recent sign-in. Run `axiom login --step-up` and retry.".to_string(),
        "AXRESOURCE_PROVIDER_LOST" => "The hosting provider no longer has this database. Only AxiomCore support can restore it; contact support.".to_string(),
        "AUTH_RECONCILING" => format!("Project sign-in is paused while AxiomCore reconciles a support restore.{retry}"),
        "AXRESOURCE_INVALID" => "Check the resource, action, generation, source and confirmation.".to_string(),
        "AXRESOURCE_DENIED" => "Only a project Owner can do this, or the environment policy does not permit it.".to_string(),
        "AXRESOURCE_CONFLICT" => "The resource changed or another operation is running. Refresh with `axiom project resources recovery` and use the current generation.".to_string(),
        "AXRESOURCE_NOT_ADMITTED" => "This database action is not available for the selected resource yet.".to_string(),
        "AXRESOURCE_PAYMENT_REQUIRED" => "An active Pro project subscription is required.".to_string(),
        "AXRESOURCE_UNAVAILABLE" => format!("The resource service is temporarily unavailable.{retry}"),
        // placement/v1 and branches-v1 codes: the shared fixed messages.
        _ => return axiom_cloud::resources::code_message(code).map(str::to_string),
    })
}

fn is_code_shaped(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

pub fn recovery_error(reply: &Reply) -> anyhow::Error {
    let resumable = reply.body.get("resumable").and_then(Value::as_bool);
    match reply
        .code()
        .and_then(|code| code_message(code, resumable, reply.retry_after).map(|m| (code, m)))
    {
        Some((code, message)) => anyhow!("{code}: {message} (HTTP {})", reply.status),
        None => anyhow!(
            "AXRESOURCE_CLOUD: request denied or unavailable (HTTP {}). Check access, admission, capacity and subscription status.",
            reply.status
        ),
    }
}

/// Terminal-safe text from a server value: no control characters, bounded.
fn clean(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(128)
        .collect()
}

fn text(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(s) if !s.is_empty() => Some(clean(s)),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn first_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| text(value, key))
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

pub struct ActionInput<'a> {
    pub resource: &'a str,
    pub action: &'a str,
    pub generation: u64,
    pub at: Option<&'a str>,
    pub backup: Option<&'a str>,
    pub confirm: Option<&'a str>,
}

fn restore_point(input: &ActionInput<'_>) -> Option<String> {
    match (input.at, input.backup) {
        (Some(at), None) => Some(at.to_string()),
        (None, Some(backup)) => Some(backup.to_string()),
        _ => None,
    }
}

/// What happens, in plain words, before the request is sent.
pub fn consequences(input: &ActionInput<'_>) -> String {
    match input.action {
        "restore" => {
            let point = match (input.at, input.backup) {
                (Some(at), _) => clean(at),
                (_, Some(backup)) => format!("the time of backup {}", clean(backup)),
                _ => "the chosen point".to_string(),
            };
            format!(
                "Application data returns to {point}. Users, passwords and sessions stay as they are now. \
                 Data written after {point}, including rows of users erased since, is replaced; the current \
                 state is kept 24 hours for support. While the database switches (about two minutes) requests \
                 answer AXRESOURCE_MOVING and sign-in pauses; signed-in users stay signed in. The restored \
                 database gets new credentials; the old ones stop working."
            )
        }
        "archive" => "The database is saved to an archive in its region and then deleted from the hosting \
             provider. Apps, sign-in and the data editor answer AXRESOURCE_ARCHIVED until an Owner resumes it: \
             an archive made by an Owner does not resume on use. Resume brings back the latest state, users \
             included. The archive is kept until 7 days after a resume or deletion."
            .to_string(),
        "unarchive" => "Resume rebuilds the database from its latest archive, with application data, users and \
             sessions as they were when it was archived. Requests answer AXRESOURCE_RESUMING until it is ready."
            .to_string(),
        "retire_previous" => "The state kept from before the last restore is deleted now instead of after 24 hours. \
             Support can no longer recover it. A new restore becomes possible."
            .to_string(),
        _ => String::new(),
    }
}

fn confirm_form(input: &ActionInput<'_>) -> String {
    match input.action {
        "restore" => format!(
            "--confirm \"restore <database name> to {}\"",
            restore_point(input)
                .map(|p| clean(&p))
                .unwrap_or_else(|| "<point or backup id>".into())
        ),
        "archive" => "--confirm \"archive <database name>\"".to_string(),
        // W6-F L-22: the phrase the server checks.
        "retire_previous" => "--confirm \"retire <database name>\"".to_string(),
        _ => "--confirm \"<phrase shown in the dashboard>\"".to_string(),
    }
}

fn valid_phrase(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}

/// Validate the flags and build the request body of the contract's API
/// section. Nothing here contacts the server.
pub fn build_body(input: &ActionInput<'_>) -> Result<Value> {
    if !is_recovery_action(input.action) {
        bail!("Unknown recovery action");
    }
    if input.generation == 0 {
        bail!("--generation must be positive");
    }
    uuid::Uuid::parse_str(input.resource).map_err(|_| anyhow!("--resource must be a UUID"))?;
    if input.action != "restore" && (input.at.is_some() || input.backup.is_some()) {
        bail!("--at and --backup are only used with --action restore");
    }
    let missing_confirm = |input: &ActionInput<'_>| {
        anyhow!(
            "{}\n\nTo go ahead, repeat the command with {}. The database name is shown by `axiom project resources recovery`.",
            consequences(input),
            confirm_form(input)
        )
    };
    match input.action {
        "restore" => {
            let source = match (input.at, input.backup) {
                (Some(at), None) => {
                    let parsed = chrono::DateTime::parse_from_rfc3339(at).map_err(|_| {
                        anyhow!("--at must be an RFC 3339 time, for example 2026-10-29T08:15:00Z")
                    })?;
                    if parsed.with_timezone(&chrono::Utc) > chrono::Utc::now() {
                        bail!("--at must be in the past");
                    }
                    json!({"kind": "point_in_time", "at": at})
                }
                (None, Some(backup)) => {
                    if backup.is_empty()
                        || backup.len() > 128
                        || !backup
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    {
                        bail!(
                            "--backup must be a backup id from `axiom project resources recovery`"
                        );
                    }
                    json!({"kind": "backup", "backup_id": backup})
                }
                _ => bail!("Restore needs exactly one of --at <RFC 3339 time> or --backup <id>"),
            };
            let point = restore_point(input).unwrap_or_default();
            let confirm = input.confirm.ok_or_else(|| missing_confirm(input))?;
            let suffix = format!(" to {point}");
            let named = confirm
                .strip_prefix("restore ")
                .and_then(|rest| rest.strip_suffix(&suffix))
                .is_some_and(|name| !name.trim().is_empty());
            if !valid_phrase(confirm) || !named {
                bail!(
                    "The confirmation does not match. Use {}.",
                    confirm_form(input)
                );
            }
            Ok(json!({"generation": input.generation, "source": source, "confirm": confirm}))
        }
        "archive" => {
            let confirm = input.confirm.ok_or_else(|| missing_confirm(input))?;
            let named = confirm
                .strip_prefix("archive ")
                .is_some_and(|name| !name.trim().is_empty());
            if !valid_phrase(confirm) || !named {
                bail!(
                    "The confirmation does not match. Use {}.",
                    confirm_form(input)
                );
            }
            Ok(json!({"generation": input.generation, "confirm": confirm}))
        }
        "retire_previous" => {
            let confirm = input.confirm.ok_or_else(|| missing_confirm(input))?;
            let named = confirm
                .strip_prefix("retire ")
                .is_some_and(|name| !name.trim().is_empty());
            if !valid_phrase(confirm) || !named {
                bail!(
                    "The confirmation does not match. Use {}.",
                    confirm_form(input)
                );
            }
            Ok(json!({"generation": input.generation, "confirm": confirm}))
        }
        "unarchive" => {
            if input.confirm.is_some() {
                bail!("Resume (--action unarchive) takes no --confirm");
            }
            Ok(json!({"generation": input.generation}))
        }
        _ => unreachable!(),
    }
}

fn started_text(action: &str, operation_id: &str) -> String {
    let next = match action {
        "restore" => "The restore runs in the background; the current database keeps serving until the switch.",
        "archive" => "The archive runs in the background.",
        "unarchive" => "The database is being rebuilt from its latest archive.",
        _ => "The request was accepted.",
    };
    format!(
        "Requested {} (operation {}). {next} Follow it with `axiom project resources recovery` or add --wait.",
        action_label(action),
        clean(operation_id)
    )
}

fn finished_text(action: &str) -> &'static str {
    match action {
        "restore" => "Restore complete. The database now serves the restored data with new credentials; the previous state is kept 24 hours for support.",
        "archive" => "Archived. The database is deleted from the provider; resume it with `--action unarchive`.",
        "unarchive" => "Resumed. The database serves its latest archived state.",
        "retire_previous" => "The previous generation is retired. A new restore is possible.",
        _ => "Done.",
    }
}

#[derive(Debug)]
pub struct Outcome {
    pub value: Value,
    pub text: String,
}

/// Run one recovery action: validate, POST with the step-up rule, and
/// optionally wait for the operation.
pub async fn run_action(
    api: &Api,
    project: &str,
    input: &ActionInput<'_>,
    operation_id: Option<&str>,
    wait: bool,
    human: bool,
) -> Result<Outcome> {
    let body = build_body(input)?;
    let key = operation_id
        .map(str::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    uuid::Uuid::parse_str(&key).map_err(|_| anyhow!("--operation-id must be a UUID"))?;
    if human {
        eprintln!("{}", consequences(input));
    }
    eprintln!("Operation idempotency key: {key}");
    let value = api
        .post_action(
            &[
                "projects",
                project,
                "resources",
                input.resource,
                input.action,
            ],
            &body,
            &key,
        )
        .await?;
    let id = value
        .get("operation_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if !wait {
        return Ok(Outcome {
            text: started_text(input.action, &id),
            value,
        });
    }
    let limit = if input.action == "retire_previous" {
        SHORT_WAIT
    } else {
        MOVE_WAIT
    };
    if human {
        eprintln!(
            "Waiting for the {} to finish (up to {} minutes)...",
            action_label(input.action),
            limit.as_secs() / 60
        );
    }
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
        let status = api.get(&["projects", project, "resources"]).await?;
        if let Some(operation) = status["operations"]
            .as_array()
            .and_then(|ops| ops.iter().find(|op| op["id"].as_str() == Some(id.as_str())))
        {
            match operation["status"].as_str() {
                Some("succeeded") => {
                    return Ok(Outcome {
                        text: finished_text(input.action).to_string(),
                        value: json!({"operation": operation}),
                    })
                }
                Some("failed") => {
                    let code = operation["failure_code"].as_str().unwrap_or_default();
                    let detail = code_message(code, None, None)
                        .map(|message| format!("{code}: {message}"))
                        .unwrap_or_else(|| {
                            if is_code_shaped(code) {
                                format!("Failure code {code}.")
                            } else {
                                "Inspect the resource for its safe failure code.".to_string()
                            }
                        });
                    bail!(
                        "The {} did not complete; the database was left as it was before. {detail}",
                        action_label(input.action)
                    );
                }
                _ => {}
            }
        }
        tokio::time::sleep(api.wait_interval).await;
    }
    bail!("The operation is still running. Run `axiom project resources --project {project} recovery --resource {}` to follow it.", input.resource)
}

// ---------------------------------------------------------------------------
// Recovery view
// ---------------------------------------------------------------------------

fn status_line(status: &str) -> Option<&'static str> {
    Some(match status {
        "ready" => "Serving.",
        "draining" | "moving" => "Switching to a restored or resumed copy; requests pause briefly.",
        "archived" => "Archived: the data is kept in an archive and the database is not running.",
        "resuming" => "Resuming from its archive.",
        "provider_waking" => "The hosting provider is waking the database group; requests answer AXRESOURCE_RESUMING.",
        "repair_required" => "Needs repair by AxiomCore.",
        _ => return None,
    })
}

/// Human rendering of `GET …/{resource_id}/recovery`. The response shape is
/// read tolerantly: missing sections are stated as unavailable, unknown
/// fields are ignored, every printed value is cleaned.
pub fn render_recovery(view: &Value) -> String {
    let mut out = Vec::new();
    let name = first_text(view, &["database_name", "database", "name"])
        .unwrap_or_else(|| "Database".to_string());
    let generation = text(view, "generation");
    let status = text(view, "status");
    out.push(format!(
        "{name}{}{}",
        generation
            .map(|g| format!(", generation {g}"))
            .unwrap_or_default(),
        status
            .as_ref()
            .map(|s| format!(", status {s}"))
            .unwrap_or_default()
    ));
    if let Some(line) = status.as_deref().and_then(status_line) {
        out.push(format!("  {line}"));
    }
    if let Some(code) = first_text(view, &["failure_code", "code"]) {
        if let Some(message) = code_message(&code, None, None) {
            out.push(format!("  {code}: {message}"));
        }
    }

    out.push(String::new());
    out.push("Point-in-time restore".to_string());
    match view.get("window").filter(|w| w.is_object()) {
        Some(window) => {
            match (
                first_text(window, &["earliest", "earliest_at", "from"]),
                first_text(window, &["latest", "latest_at", "to"]),
            ) {
                (Some(earliest), Some(latest)) => out.push(format!(
                    "  Any second from {earliest} to {latest}: --at <RFC 3339 time>"
                )),
                (Some(earliest), None) => out.push(format!(
                    "  Any second from {earliest} until now: --at <RFC 3339 time>"
                )),
                _ => out.push("  Not available now.".to_string()),
            }
        }
        None => out.push("  Not available now.".to_string()),
    }

    out.push(String::new());
    out.push("Daily backups (kept 7 days): --backup <id>".to_string());
    match view.get("backups").and_then(Value::as_array) {
        Some(backups) if !backups.is_empty() => {
            for backup in backups {
                let id = text(backup, "id").unwrap_or_else(|| "?".into());
                let kind = text(backup, "kind").unwrap_or_else(|| "daily".into());
                let taken =
                    first_text(backup, &["taken_at", "created_at"]).unwrap_or_else(|| "?".into());
                let size = backup
                    .get("size_bytes")
                    .and_then(Value::as_u64)
                    .map(|bytes| format!(", {}", human_bytes(bytes)))
                    .unwrap_or_default();
                let verified = match backup.get("verified_at").or_else(|| backup.get("verified")) {
                    Some(Value::Null) | Some(Value::Bool(false)) => ", not verified",
                    Some(_) => ", verified",
                    None => "",
                };
                out.push(format!("  {id}  {kind}  {taken}{size}{verified}"));
            }
        }
        _ => out.push("  None yet.".to_string()),
    }

    if let Some(previous) = view.get("previous").filter(|p| p.is_object()) {
        out.push(String::new());
        let generation = text(previous, "generation").unwrap_or_else(|| "?".into());
        let until = first_text(previous, &["retire_after", "kept_until"])
            .unwrap_or_else(|| "24 hours after the restore".into());
        out.push(format!(
            "The state from before the last restore (generation {generation}) is kept until {until} for support. \
             A new restore is refused until then unless you retire it early with --action retire_previous."
        ));
    }

    if let Some(archive) = view.get("archive").filter(|a| a.is_object()) {
        let state = text(archive, "state");
        if state.as_deref().is_some_and(|s| s != "none") {
            out.push(String::new());
            let origin = text(archive, "origin");
            let taken = first_text(archive, &["taken_at", "archived_at"]);
            out.push(format!(
                "Archive: {}{}{}",
                state.unwrap_or_default(),
                origin
                    .as_deref()
                    .map(|o| format!(
                        ", made by {}",
                        if o == "owner" {
                            "an Owner"
                        } else {
                            "the inactivity rule"
                        }
                    ))
                    .unwrap_or_default(),
                taken.map(|t| format!(", taken {t}")).unwrap_or_default()
            ));
            out.push(match origin.as_deref() {
                Some("owner") => {
                    "  It does not resume on use; resume it with --action unarchive.".to_string()
                }
                _ => "  It resumes on next use, or now with --action unarchive.".to_string(),
            });
        }
    }

    if let Some(inactivity) = view.get("inactivity").filter(|i| i.is_object()) {
        if let Some(archives_at) = first_text(inactivity, &["archives_at", "archive_at"]) {
            out.push(String::new());
            let last = first_text(inactivity, &["last_activity_at"]);
            out.push(format!(
                "Inactivity: {}archives on {archives_at} if unused (Free databases unused for 30 days are archived; any use resets this).",
                last.map(|l| format!("last used {l}; ")).unwrap_or_default()
            ));
        }
    }

    out.push(String::new());
    out.push("Recent moves".to_string());
    match view.get("moves").and_then(Value::as_array) {
        Some(moves) if !moves.is_empty() => {
            for item in moves.iter().take(10) {
                let kind = text(item, "kind").unwrap_or_else(|| "?".into());
                let kind = if kind == "unarchive" {
                    "resume".to_string()
                } else {
                    kind
                };
                let state = text(item, "state").unwrap_or_else(|| "?".into());
                let when = first_text(item, &["activated_at", "started_at"]).unwrap_or_default();
                let generations = match (text(item, "from_generation"), text(item, "to_generation"))
                {
                    (Some(from), Some(to)) => format!(" generation {from} to {to}"),
                    _ => String::new(),
                };
                let code = text(item, "code")
                    .filter(|c| is_code_shaped(c))
                    .map(|c| format!(" ({c})"))
                    .unwrap_or_default();
                out.push(format!("  {when}  {kind}{generations}  {state}{code}"));
            }
        }
        _ => out.push("  None.".to_string()),
    }
    out.join("\n")
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub async fn view(api: &Api, project: &str, resource: &str) -> Result<Value> {
    uuid::Uuid::parse_str(resource).map_err(|_| anyhow!("--resource must be a UUID"))?;
    api.get(&["projects", project, "resources", resource, "recovery"])
        .await
}

// ---------------------------------------------------------------------------
// Entry points used by `resources.rs`
// ---------------------------------------------------------------------------

pub async fn handle_view(project: &str, resource: &str, as_json: bool) -> Result<()> {
    let api = Api::from_profile()?;
    let value = view(&api, project, resource).await?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{}", render_recovery(&value));
    }
    Ok(())
}

pub async fn handle_action(
    project: &str,
    input: &ActionInput<'_>,
    operation_id: Option<&str>,
    wait: bool,
    as_json: bool,
) -> Result<()> {
    // Validate before loading the session so flag mistakes never need a login.
    build_body(input)?;
    let api = Api::from_profile()?;
    let outcome = run_action(&api, project, input, operation_id, wait, !as_json).await?;
    if as_json {
        println!("{}", serde_json::to_string_pretty(&outcome.value)?);
    } else {
        println!("{}", outcome.text);
    }
    Ok(())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod data_tests;
